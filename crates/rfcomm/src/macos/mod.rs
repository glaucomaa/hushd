use std::collections::VecDeque;
use std::ffi::c_void;
use std::io;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AnyThread, DefinedClass};
use objc2_core_foundation::{kCFRunLoopDefaultMode, CFRunLoop};
use objc2_foundation::NSString;
use objc2_io_bluetooth::{
    IOBluetoothDevice, IOBluetoothDeviceAsyncCallbacks, IOBluetoothRFCOMMChannel,
    IOBluetoothRFCOMMChannelDelegate,
};

pub mod discovery;
pub mod sdp;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("bluetooth I/O: {0}")]
    Io(#[from] io::Error),
    #[error("invalid bluetooth address: {0}")]
    Address(String),
    #[error("no GAIA channel answered on {address}")]
    NoChannel { address: String },
    #[error("IOBluetooth: {0}")]
    Bluetooth(String),
}

pub fn parse_address(text: &str) -> Result<[u8; 6], Error> {
    let normalized = text.replace('-', ":");
    let parts: Vec<&str> = normalized.split(':').collect();
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

#[derive(Debug, Default)]
struct State {
    incoming: VecDeque<u8>,
    closed: bool,
    open_status: Option<i32>,
    sdp_status: Option<i32>,
}

#[derive(Debug, Default)]
struct Shared {
    state: Mutex<State>,
    readable: Condvar,
}

impl Shared {
    fn update(&self, update: impl FnOnce(&mut State)) {
        if let Ok(mut state) = self.state.lock() {
            update(&mut state);
            self.readable.notify_all();
        }
    }
}

#[derive(Debug)]
struct DelegateIvars {
    shared: Arc<Shared>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[ivars = DelegateIvars]
    struct ChannelDelegate;

    unsafe impl NSObjectProtocol for ChannelDelegate {}

    unsafe impl IOBluetoothRFCOMMChannelDelegate for ChannelDelegate {
        #[unsafe(method(rfcommChannelData:data:length:))]
        fn did_receive(
            &self,
            _channel: Option<&IOBluetoothRFCOMMChannel>,
            data: *mut c_void,
            length: usize,
        ) {
            if data.is_null() {
                return;
            }
            // SAFETY: IOBluetooth owns a readable buffer of `length` bytes for this callback.
            let bytes = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), length) };
            self.ivars().shared.update(|state| {
                state.incoming.extend(bytes);
            });
        }

        #[unsafe(method(rfcommChannelOpenComplete:status:))]
        fn did_open(&self, _channel: Option<&IOBluetoothRFCOMMChannel>, status: i32) {
            self.ivars().shared.update(|state| {
                state.open_status = Some(status);
            });
        }

        #[unsafe(method(rfcommChannelClosed:))]
        fn did_close(&self, _channel: Option<&IOBluetoothRFCOMMChannel>) {
            self.ivars().shared.update(|state| {
                state.closed = true;
            });
        }
    }

    unsafe impl IOBluetoothDeviceAsyncCallbacks for ChannelDelegate {
        #[unsafe(method(sdpQueryComplete:status:))]
        fn did_query_sdp(&self, _device: Option<&IOBluetoothDevice>, status: i32) {
            self.ivars().shared.update(|state| {
                state.sdp_status = Some(status);
            });
        }
    }
);

impl ChannelDelegate {
    fn new(shared: Arc<Shared>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(DelegateIvars { shared });
        // SAFETY: the superclass is NSObject and its init signature is unchanged.
        unsafe { msg_send![super(this), init] }
    }
}

fn run_loop_slice() {
    // SAFETY: the default mode is a process-wide constant supplied by CoreFoundation.
    let mode = unsafe { kCFRunLoopDefaultMode };
    CFRunLoop::run_in_mode(mode, 0.05, true);
}

pub fn pump_run_loop() {
    // SAFETY: the default mode is a process-wide constant supplied by CoreFoundation.
    let mode = unsafe { kCFRunLoopDefaultMode };
    CFRunLoop::run_in_mode(mode, 0.1, true);
}

fn bluetooth_error(operation: &str, status: i32) -> Error {
    Error::Bluetooth(format!("{operation} failed with IOReturn {status:#010x}"))
}

pub(crate) fn device(address: [u8; 6]) -> Result<Retained<IOBluetoothDevice>, Error> {
    let text = NSString::from_str(&format_address(&address));
    // SAFETY: the string remains alive for the call and uses the documented address format.
    unsafe { IOBluetoothDevice::deviceWithAddressString(Some(&text)) }.ok_or_else(|| {
        Error::Bluetooth(format!(
            "{} is not paired with this Mac",
            format_address(&address)
        ))
    })
}

enum Command {
    Write(Vec<u8>, Sender<Result<(), String>>),
    Close,
}

pub struct Link {
    shared: Arc<Shared>,
    commands: Sender<Command>,
}

impl Link {
    pub fn connect(address: [u8; 6], channel: u8, timeout: Duration) -> Result<Self, Error> {
        let shared = Arc::new(Shared::default());
        let (commands, receiver) = mpsc::channel();
        let (ready, opened) = mpsc::channel();
        let runtime_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("m4-rfcomm".into())
            .spawn(move || {
                objc2::rc::autoreleasepool(|_| {
                    runtime(address, channel, timeout, runtime_shared, receiver, ready);
                });
            })
            .map_err(Error::Io)?;
        match opened.recv_timeout(timeout + Duration::from_millis(250)) {
            Ok(Ok(())) => Ok(Self { shared, commands }),
            Ok(Err(message)) => Err(Error::Bluetooth(message)),
            Err(_) => Err(Error::Io(io::Error::new(
                io::ErrorKind::TimedOut,
                "rfcomm connect timed out",
            ))),
        }
    }

    pub fn send(&self, bytes: &[u8]) -> io::Result<()> {
        if self
            .shared
            .state
            .lock()
            .map(|state| state.closed)
            .unwrap_or(true)
        {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "link closed"));
        }
        let (reply, result) = mpsc::channel();
        self.commands
            .send(Command::Write(bytes.to_vec(), reply))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "link closed"))?;
        result
            .recv_timeout(Duration::from_secs(3))
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "send timed out"))?
            .map_err(io::Error::other)
    }

    pub fn recv(&self, buffer: &mut [u8], timeout: Duration) -> io::Result<Option<usize>> {
        let deadline = Instant::now() + timeout;
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| io::Error::other("rfcomm state is poisoned"))?;
        loop {
            if !state.incoming.is_empty() {
                let count = buffer.len().min(state.incoming.len());
                for slot in &mut buffer[..count] {
                    *slot = state.incoming.pop_front().expect("queue is not empty");
                }
                return Ok(Some(count));
            }
            if state.closed {
                return Ok(Some(0));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            let (next, wait) = self
                .shared
                .readable
                .wait_timeout(state, remaining)
                .map_err(|_| io::Error::other("rfcomm state is poisoned"))?;
            state = next;
            if wait.timed_out() && state.incoming.is_empty() {
                return Ok(None);
            }
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Close);
    }
}

fn runtime(
    address: [u8; 6],
    channel_id: u8,
    timeout: Duration,
    shared: Arc<Shared>,
    commands: mpsc::Receiver<Command>,
    ready: Sender<Result<(), String>>,
) {
    let result = open_channel(address, channel_id, timeout, Arc::clone(&shared));
    let (_device, _delegate, mut channel) = match result {
        Ok(parts) => parts,
        Err(error) => {
            shared.update(|state| state.closed = true);
            let _ = ready.send(Err(error.to_string()));
            return;
        }
    };
    if ready.send(Ok(())).is_err() {
        if let Some(channel) = channel.take() {
            // SAFETY: the channel is owned by this run-loop thread.
            unsafe { channel.closeChannel() };
        }
        return;
    }

    loop {
        while let Ok(command) = commands.try_recv() {
            match command {
                Command::Write(bytes, reply) => {
                    let result = channel
                        .as_ref()
                        .ok_or_else(|| "link closed".to_owned())
                        .and_then(|channel| write(channel, &bytes));
                    let _ = reply.send(result);
                }
                Command::Close => {
                    if let Some(channel) = channel.take() {
                        // SAFETY: the channel and delegate live on this run-loop thread.
                        unsafe { channel.closeChannel() };
                    }
                    shared.update(|state| state.closed = true);
                    return;
                }
            }
        }
        if shared
            .state
            .lock()
            .map(|state| state.closed)
            .unwrap_or(true)
        {
            if let Some(channel) = channel.take() {
                // SAFETY: the channel is owned by this run-loop thread.
                unsafe { channel.closeChannel() };
            }
            return;
        }
        run_loop_slice();
    }
}

type OpenParts = (
    Retained<IOBluetoothDevice>,
    Retained<ChannelDelegate>,
    Option<Retained<IOBluetoothRFCOMMChannel>>,
);

fn open_channel(
    address: [u8; 6],
    channel_id: u8,
    timeout: Duration,
    shared: Arc<Shared>,
) -> Result<OpenParts, Error> {
    let device = device(address)?;
    let delegate = ChannelDelegate::new(Arc::clone(&shared));
    let mut channel = None;
    // SAFETY: all objects stay on this run-loop thread until the channel closes.
    let status = unsafe {
        device.openRFCOMMChannelAsync_withChannelID_delegate(
            Some(&mut channel),
            channel_id,
            Some(
                ProtocolObject::<dyn IOBluetoothRFCOMMChannelDelegate>::from_ref(&*delegate)
                    .as_ref(),
            ),
        )
    };
    if status != 0 {
        return Err(bluetooth_error("open RFCOMM channel", status));
    }
    if let Some(channel) = &channel {
        // SAFETY: reused channels need their delegate replaced explicitly.
        let status = unsafe {
            channel.setDelegate(Some(
                ProtocolObject::<dyn IOBluetoothRFCOMMChannelDelegate>::from_ref(&*delegate)
                    .as_ref(),
            ))
        };
        if status != 0 {
            return Err(bluetooth_error("set RFCOMM delegate", status));
        }
    }
    if channel
        .as_ref()
        .is_some_and(|channel| unsafe { channel.isOpen() })
    {
        return Ok((device, delegate, channel));
    }
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = shared
            .state
            .lock()
            .map_err(|_| Error::Bluetooth("channel state is poisoned".into()))?
            .open_status
        {
            if status != 0 {
                return Err(bluetooth_error("open RFCOMM channel", status));
            }
            break;
        }
        if Instant::now() >= deadline {
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::TimedOut,
                "rfcomm open timed out",
            )));
        }
        run_loop_slice();
    }
    if channel.is_none() {
        return Err(Error::Bluetooth(
            "open RFCOMM channel returned no channel".into(),
        ));
    }
    Ok((device, delegate, channel))
}

fn write(channel: &IOBluetoothRFCOMMChannel, bytes: &[u8]) -> Result<(), String> {
    // SAFETY: the MTU is queried on the run-loop thread that owns the channel.
    let mtu = unsafe { channel.getMTU() } as usize;
    if bytes.len() > mtu || bytes.len() > u16::MAX as usize {
        return Err(format!(
            "GAIA frame is {} bytes, channel MTU is {mtu}",
            bytes.len()
        ));
    }
    // SAFETY: the byte slice is valid for the duration of this synchronous write.
    let status =
        unsafe { channel.writeSync_length(bytes.as_ptr().cast_mut().cast(), bytes.len() as u16) };
    if status == 0 {
        Ok(())
    } else {
        Err(format!("RFCOMM write failed with IOReturn {status:#010x}"))
    }
}

pub fn connect_any(
    address: [u8; 6],
    channels: &[u8],
    timeout: Duration,
) -> Result<(Link, u8), Error> {
    let mut last = None;
    for attempt in 0..4 {
        for &channel in channels {
            match Link::connect(address, channel, timeout) {
                Ok(link) => return Ok((link, channel)),
                Err(error) => last = Some(error),
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
