use std::io;
use std::os::unix::io::RawFd;
use std::time::{Duration, Instant};

pub mod bluez;
pub mod sdp;

const AF_BLUETOOTH: i32 = 31;
const BTPROTO_RFCOMM: i32 = 3;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("bluetooth socket: {0}")]
    Io(#[from] io::Error),
    #[error("invalid bluetooth address: {0}")]
    Address(String),
    #[error("no GAIA channel answered on {address}")]
    NoChannel { address: String },
}

/// `sockaddr_rc` from bluetooth/rfcomm.h.
#[repr(C)]
struct SockaddrRc {
    rc_family: u16,
    rc_bdaddr: [u8; 6],
    rc_channel: u8,
}

/// bdaddr_t is stored least-significant byte first, the reverse of how the
/// address is printed.
fn kernel_address(address: [u8; 6]) -> [u8; 6] {
    let mut reversed = address;
    reversed.reverse();
    reversed
}

pub fn parse_address(text: &str) -> Result<[u8; 6], Error> {
    let parts: Vec<&str> = text.split(':').collect();
    if parts.len() != 6 {
        return Err(Error::Address(text.to_owned()));
    }
    let mut address = [0u8; 6];
    for (slot, part) in address.iter_mut().zip(parts) {
        *slot = u8::from_str_radix(part, 16).map_err(|_| Error::Address(text.to_owned()))?;
    }
    Ok(address)
}

pub fn format_address(address: &[u8; 6]) -> String {
    address
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

pub struct Link {
    fd: RawFd,
}

impl Link {
    pub fn connect(address: [u8; 6], channel: u8, timeout: Duration) -> Result<Self, Error> {
        // SAFETY: plain syscalls on a descriptor we own.
        unsafe {
            let fd = libc::socket(
                AF_BLUETOOTH,
                libc::SOCK_STREAM | libc::SOCK_CLOEXEC,
                BTPROTO_RFCOMM,
            );
            if fd < 0 {
                return Err(io::Error::last_os_error().into());
            }
            let link = Link { fd };

            let flags = libc::fcntl(fd, libc::F_GETFL);
            if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
                return Err(io::Error::last_os_error().into());
            }

            let sockaddr = SockaddrRc {
                rc_family: AF_BLUETOOTH as u16,
                rc_bdaddr: kernel_address(address),
                rc_channel: channel,
            };
            let rc = libc::connect(
                fd,
                &sockaddr as *const SockaddrRc as *const libc::sockaddr,
                std::mem::size_of::<SockaddrRc>() as libc::socklen_t,
            );
            if rc != 0 {
                let err = io::Error::last_os_error();
                if err.raw_os_error() != Some(libc::EINPROGRESS) {
                    return Err(err.into());
                }
                let mut pollfd = libc::pollfd {
                    fd,
                    events: libc::POLLOUT,
                    revents: 0,
                };
                let millis = timeout.as_millis().min(i32::MAX as u128) as i32;
                let ready = libc::poll(&mut pollfd, 1, millis);
                if ready == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "rfcomm connect timed out",
                    )
                    .into());
                }
                if ready < 0 {
                    return Err(io::Error::last_os_error().into());
                }
                let mut error: libc::c_int = 0;
                let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
                if libc::getsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_ERROR,
                    &mut error as *mut _ as *mut libc::c_void,
                    &mut len,
                ) < 0
                {
                    return Err(io::Error::last_os_error().into());
                }
                if error != 0 {
                    return Err(io::Error::from_raw_os_error(error).into());
                }
            }
            libc::fcntl(fd, libc::F_SETFL, flags);
            Ok(link)
        }
    }

    pub fn send(&self, bytes: &[u8]) -> io::Result<()> {
        let mut written = 0;
        while written < bytes.len() {
            // SAFETY: the slice is valid for `bytes.len() - written` bytes.
            let rc = unsafe {
                libc::send(
                    self.fd,
                    bytes[written..].as_ptr() as *const libc::c_void,
                    bytes.len() - written,
                    libc::MSG_NOSIGNAL,
                )
            };
            if rc < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                if err.kind() == io::ErrorKind::WouldBlock {
                    if !self.wait(libc::POLLOUT, Duration::from_secs(2))? {
                        return Err(io::Error::new(io::ErrorKind::TimedOut, "send timed out"));
                    }
                    continue;
                }
                return Err(err);
            }
            written += rc as usize;
        }
        Ok(())
    }

    pub fn recv(&self, buf: &mut [u8], timeout: Duration) -> io::Result<Option<usize>> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if !self.wait(libc::POLLIN, remaining)? {
                return Ok(None);
            }
            // SAFETY: the buffer is valid for `buf.len()` bytes.
            let rc = unsafe {
                libc::recv(
                    self.fd,
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                    libc::MSG_DONTWAIT,
                )
            };
            if rc < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                if err.kind() == io::ErrorKind::WouldBlock {
                    continue;
                }
                return Err(err);
            }
            return Ok(Some(rc as usize));
        }
    }

    /// RFCOMM exposes `ETIMEDOUT` as a hung-up link; callers treat it as EOF.
    pub fn is_hopeless(error: &io::Error) -> bool {
        matches!(
            error.raw_os_error(),
            Some(libc::ETIMEDOUT) | Some(libc::ECONNRESET) | Some(libc::ENOTCONN)
        )
    }

    fn wait(&self, events: i16, timeout: Duration) -> io::Result<bool> {
        let mut pollfd = libc::pollfd {
            fd: self.fd,
            events,
            revents: 0,
        };
        let millis = timeout.as_millis().min(i32::MAX as u128) as i32;
        // SAFETY: one valid pollfd.
        let rc = unsafe { libc::poll(&mut pollfd, 1, millis) };
        if rc < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                return Ok(false);
            }
            return Err(err);
        }
        Ok(rc > 0)
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        // SAFETY: closing the descriptor we own.
        unsafe {
            libc::close(self.fd);
        }
    }
}

/// The headset needs roughly half a second to tear the previous link down;
/// connecting earlier fails with EBUSY.
pub fn connect_any(
    address: [u8; 6],
    channels: &[u8],
    timeout: Duration,
) -> Result<(Link, u8), Error> {
    let mut last: Option<Error> = None;
    for attempt in 0..4 {
        for &channel in channels {
            match Link::connect(address, channel, timeout) {
                Ok(link) => return Ok((link, channel)),
                Err(err) => last = Some(err),
            }
        }
        if attempt < 3 {
            std::thread::sleep(Duration::from_millis(500));
        }
    }
    Err(last.unwrap_or(Error::NoChannel {
        address: format_address(&address),
    }))
}
