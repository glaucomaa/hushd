use std::fmt;

use thiserror::Error;

pub const START: u8 = 0xff;
pub const VERSION_CONTROL: u8 = 0x03;

const FLAG_CHECKSUM: u8 = 0x01;
const FLAG_LEN16: u8 = 0x02;
const TYPE_MASK: u16 = 0x0180;
const MAX_PAYLOAD: usize = 254;
const MAX_WIDE_PAYLOAD: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Command,
    Notification,
    Response,
    Error,
}

impl Kind {
    pub fn of(command: u16) -> Self {
        match (command & TYPE_MASK) >> 7 {
            0 => Self::Command,
            1 => Self::Notification,
            2 => Self::Response,
            _ => Self::Error,
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Command => "command",
            Self::Notification => "notification",
            Self::Response => "response",
            Self::Error => "error",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub vendor: u16,
    pub command: u16,
    pub payload: Vec<u8>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EncodeError {
    #[error("payload of {0} bytes does not fit in one control frame")]
    TooLong(usize),
}

impl Frame {
    pub fn new(vendor: u16, command: u16, payload: impl Into<Vec<u8>>) -> Self {
        Self {
            vendor,
            command,
            payload: payload.into(),
        }
    }

    pub fn kind(&self) -> Kind {
        Kind::of(self.command)
    }

    pub fn base(&self) -> u16 {
        self.command & !TYPE_MASK
    }

    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        if self.payload.len() > MAX_PAYLOAD {
            return Err(EncodeError::TooLong(self.payload.len()));
        }
        let mut out = Vec::with_capacity(8 + self.payload.len());
        out.extend_from_slice(&[START, VERSION_CONTROL, 0x00, self.payload.len() as u8]);
        out.extend_from_slice(&self.vendor.to_be_bytes());
        out.extend_from_slice(&self.command.to_be_bytes());
        out.extend_from_slice(&self.payload);
        Ok(out)
    }
}

impl fmt::Display for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04x}:{:04x} {} ",
            self.vendor,
            self.command,
            self.kind()
        )?;
        for byte in &self.payload {
            write!(f, "{byte:02x} ")?;
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct Deframer {
    buf: Vec<u8>,
    discarded: u64,
    bad: u64,
}

impl Deframer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    pub fn discarded(&self) -> u64 {
        self.discarded
    }

    pub fn bad_frames(&self) -> u64 {
        self.bad
    }

    pub fn pending(&self) -> usize {
        self.buf.len()
    }

    pub fn next_frame(&mut self) -> Option<Frame> {
        loop {
            let Some(start) = self.buf.iter().position(|&b| b == START) else {
                self.discarded += self.buf.len() as u64;
                self.buf.clear();
                return None;
            };
            if start > 0 {
                self.discarded += start as u64;
                self.buf.drain(..start);
            }
            if self.buf.len() < 4 {
                return None;
            }
            let version = self.buf[1];
            let flags = self.buf[2];
            let known_version = matches!(version, 0x01 | VERSION_CONTROL | 0x04);
            if !known_version || flags & !(FLAG_CHECKSUM | FLAG_LEN16) != 0 {
                self.bad += 1;
                self.drop_byte();
                continue;
            }
            let wide = flags & FLAG_LEN16 != 0;
            let header = if wide { 5 } else { 4 };
            if self.buf.len() < header {
                return None;
            }
            let len = if wide {
                u16::from_be_bytes([self.buf[3], self.buf[4]]) as usize
            } else {
                self.buf[3] as usize
            };
            // A bogus length word must not grow the buffer forever.
            let limit = if wide { MAX_WIDE_PAYLOAD } else { MAX_PAYLOAD };
            if len > limit {
                self.bad += 1;
                self.drop_byte();
                continue;
            }
            let checksum = usize::from(flags & FLAG_CHECKSUM != 0);
            let total = header + 4 + len + checksum;
            if self.buf.len() < total {
                return None;
            }
            let vendor = u16::from_be_bytes([self.buf[header], self.buf[header + 1]]);
            let command = u16::from_be_bytes([self.buf[header + 2], self.buf[header + 3]]);
            let payload = self.buf[header + 4..header + 4 + len].to_vec();
            self.buf.drain(..total);
            return Some(Frame {
                vendor,
                command,
                payload,
            });
        }
    }

    fn drop_byte(&mut self) {
        self.buf.drain(..1);
        self.discarded += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_battery_request() {
        let frame = Frame::new(0x0495, 0x0603, Vec::new());
        assert_eq!(
            frame.encode().unwrap(),
            vec![0xff, 0x03, 0x00, 0x00, 0x04, 0x95, 0x06, 0x03]
        );
        assert_eq!(frame.kind(), Kind::Command);
    }

    #[test]
    fn splits_commands_from_ids() {
        assert_eq!(Kind::of(0x0603), Kind::Command);
        assert_eq!(Kind::of(0x0683), Kind::Notification);
        assert_eq!(Kind::of(0x0703), Kind::Response);
        assert_eq!(Kind::of(0x0783), Kind::Error);
        assert_eq!(Frame::new(0x0495, 0x0703, vec![1]).base(), 0x0603);
    }

    #[test]
    fn resyncs_after_garbage() {
        let raw = [
            0x00, 0x11, 0xff, 0x03, 0x7f, 0xff, 0x03, 0x00, 0x01, 0x04, 0x95, 0x07, 0x03, 0x64,
        ];
        let mut deframer = Deframer::new();
        deframer.push(&raw);
        let frame = deframer.next_frame().unwrap();
        assert_eq!(frame.command, 0x0703);
        assert_eq!(frame.payload, vec![0x64]);
        assert_eq!(deframer.discarded(), 5);
        assert_eq!(deframer.bad_frames(), 1);
    }

    #[test]
    fn handles_split_reads() {
        let raw = Frame::new(0x0495, 0x1201, vec![0, 3, 0, 38, 0, 3])
            .encode()
            .unwrap();
        let mut deframer = Deframer::new();
        let mut frames = Vec::new();
        for chunk in raw.chunks(3) {
            deframer.push(chunk);
            while let Some(frame) = deframer.next_frame() {
                frames.push(frame);
            }
        }
        assert_eq!(frames.len(), 1, "exactly one frame across split reads");
        assert_eq!(frames[0].command, 0x1201);
        assert_eq!(frames[0].payload, vec![0, 3, 0, 38, 0, 3]);
    }

    #[test]
    fn strips_checksum_byte() {
        let mut raw = Frame::new(0x0495, 0x0603, vec![0x64]).encode().unwrap();
        raw[2] = FLAG_CHECKSUM;
        raw.push(0x00);
        let mut deframer = Deframer::new();
        deframer.push(&raw);
        assert_eq!(deframer.next_frame().unwrap().payload, vec![0x64]);
    }

    #[test]
    fn rejects_oversized_payload() {
        let frame = Frame::new(0x0495, 0x1001, vec![0; 255]);
        assert_eq!(frame.encode(), Err(EncodeError::TooLong(255)));
    }
}
