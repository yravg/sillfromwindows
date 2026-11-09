//! Framing: Sources/StreamProtocol/StreamMessage.swift and iOSClient/MessageReader.swift.

use std::fmt;

/// `StreamMessage.headerLength` (StreamMessage.swift:105).
pub const HEADER_LEN: usize = 14;

/// Caps on what a header may announce (StreamMessage.swift:107-117).
pub const MAX_CLIENT_PAYLOAD: usize = 1 << 20;
pub const MAX_PAIRING_PAYLOAD: usize = 4096;
pub const MAX_FRAME_PAYLOAD: usize = 32 << 20;
pub const MAX_OTHER_HOST_PAYLOAD: usize = 4 << 20;

/// Which way a kind travels (comments in StreamMessage.swift:5-95).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    HostToDevice,
    DeviceToHost,
    Both,
}

/// `StreamMessageKind` (StreamMessage.swift:5-95). A number this build does not know is
/// `Unknown(n)`, never an error: the reader skips it by length (StreamMessage.swift:143-145).
/// 29-31 and 33-42 are unused by the Mac host (reserved for another client) and parse as `Unknown`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MessageKind {
    ParameterSets,
    Frame,
    WindowList,
    Thumbnail,
    AppIcon,
    AppList,
    SelectSource,
    LaunchApp,
    Input,
    Viewport,
    Ping,
    Pong,
    ClientStats,
    Tick,
    CursorShape,
    WindowCommand,
    HostSettings,
    ChangeSettings,
    MacInfo,
    PairRequest,
    PairResult,
    PairingWanted,
    Goodbye,
    Hello,
    MacMenu,
    PressMenuItem,
    MacPointer,
    FetchMenu,
    Gesture,
    Clipboard,
    Audio,
    KeyframeRequest,
    Unknown(u8),
}

impl MessageKind {
    /// The kind for a wire number (`StreamMessageKind(rawValue:) ?? .unknown`).
    pub fn from_u8(n: u8) -> MessageKind {
        use MessageKind::*;
        match n {
            0 => ParameterSets,
            1 => Frame,
            2 => WindowList,
            3 => Thumbnail,
            4 => AppIcon,
            5 => AppList,
            6 => SelectSource,
            7 => LaunchApp,
            8 => Input,
            9 => Viewport,
            10 => Ping,
            11 => Pong,
            12 => ClientStats,
            13 => Tick,
            14 => CursorShape,
            15 => WindowCommand,
            16 => HostSettings,
            17 => ChangeSettings,
            18 => MacInfo,
            19 => PairRequest,
            20 => PairResult,
            21 => PairingWanted,
            22 => Goodbye,
            23 => Hello,
            24 => MacMenu,
            25 => PressMenuItem,
            26 => MacPointer,
            27 => FetchMenu,
            28 => Gesture,
            32 => Clipboard,
            43 => Audio,
            44 => KeyframeRequest,
            other => Unknown(other),
        }
    }

    /// The wire number. `Unknown(n)` keeps the number it was read with.
    pub fn as_u8(self) -> u8 {
        use MessageKind::*;
        match self {
            ParameterSets => 0,
            Frame => 1,
            WindowList => 2,
            Thumbnail => 3,
            AppIcon => 4,
            AppList => 5,
            SelectSource => 6,
            LaunchApp => 7,
            Input => 8,
            Viewport => 9,
            Ping => 10,
            Pong => 11,
            ClientStats => 12,
            Tick => 13,
            CursorShape => 14,
            WindowCommand => 15,
            HostSettings => 16,
            ChangeSettings => 17,
            MacInfo => 18,
            PairRequest => 19,
            PairResult => 20,
            PairingWanted => 21,
            Goodbye => 22,
            Hello => 23,
            MacMenu => 24,
            PressMenuItem => 25,
            MacPointer => 26,
            FetchMenu => 27,
            Gesture => 28,
            Clipboard => 32,
            Audio => 43,
            KeyframeRequest => 44,
            Unknown(n) => n,
        }
    }

    /// Direction per the comments on each case; `None` for `Unknown`.
    pub fn direction(self) -> Option<Direction> {
        use Direction::*;
        use MessageKind::*;
        Some(match self {
            ParameterSets | Frame | WindowList | Thumbnail | AppIcon | AppList | Pong | Tick
            | CursorShape | HostSettings | MacInfo | PairResult | Goodbye | MacMenu
            | MacPointer | Audio => HostToDevice,
            SelectSource | LaunchApp | Input | Viewport | Ping | ClientStats | WindowCommand
            | ChangeSettings | PairRequest | PairingWanted | Hello | PressMenuItem | FetchMenu
            | Gesture | KeyframeRequest => DeviceToHost,
            Clipboard => Both,
            Unknown(_) => return None,
        })
    }
}

/// `StreamHeader` (StreamMessage.swift:97-102).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Header {
    pub kind: MessageKind,
    /// Host wall clock, seconds since 1970 (a device's ping carries its own clock).
    pub timestamp: f64,
    pub is_keyframe: bool,
    pub payload_length: usize,
}

impl Header {
    /// `StreamMessage.parseHeader` (StreamMessage.swift:141-150): `None` under 14 bytes; the key
    /// flag is true only for exactly 1; an unknown kind is kept as `Unknown`.
    pub fn parse(data: &[u8]) -> Option<Header> {
        if data.len() < HEADER_LEN {
            return None;
        }
        let ts = u64::from_be_bytes(data[1..9].try_into().ok()?);
        let len = u32::from_be_bytes(data[10..14].try_into().ok()?);
        Some(Header {
            kind: MessageKind::from_u8(data[0]),
            timestamp: f64::from_bits(ts),
            is_keyframe: data[9] == 1,
            payload_length: len as usize,
        })
    }

    /// The 14 bytes (StreamMessage.swift:131-139). `Err` when the length does not fit a u32.
    pub fn to_bytes(&self) -> Result<[u8; HEADER_LEN], FrameError> {
        let len = u32::try_from(self.payload_length).map_err(|_| FrameError::PayloadTooLarge)?;
        let mut b = [0u8; HEADER_LEN];
        b[0] = self.kind.as_u8();
        b[1..9].copy_from_slice(&self.timestamp.to_bits().to_be_bytes());
        b[9] = u8::from(self.is_keyframe);
        b[10..14].copy_from_slice(&len.to_be_bytes());
        Ok(b)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// A payload longer than the header's UInt32 length can say.
    PayloadTooLarge,
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "payload does not fit a 32-bit length")
    }
}
impl std::error::Error for FrameError {}

/// `StreamMessage` (StreamMessage.swift:104-139).
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub kind: MessageKind,
    pub timestamp: f64,
    pub is_keyframe: bool,
    pub payload: Vec<u8>,
}

impl Message {
    pub fn new(kind: MessageKind, timestamp: f64, is_keyframe: bool, payload: Vec<u8>) -> Self {
        Message {
            kind,
            timestamp,
            is_keyframe,
            payload,
        }
    }

    /// `serialized()`: header then payload.
    pub fn serialize(&self) -> Result<Vec<u8>, FrameError> {
        let header = Header {
            kind: self.kind,
            timestamp: self.timestamp,
            is_keyframe: self.is_keyframe,
            payload_length: self.payload.len(),
        }
        .to_bytes()?;
        let mut out = Vec::with_capacity(HEADER_LEN + self.payload.len());
        out.extend_from_slice(&header);
        out.extend_from_slice(&self.payload);
        Ok(out)
    }
}

/// Per-direction caps a reader enforces on a header's announced length. Over the cap the reading
/// ends with [`ReadError::TooBig`] before any payload is read (MessageReader.swift:86-91).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps {
    pub frame: usize,
    pub other: usize,
}

impl Caps {
    /// A device reading the host: frames 32 MiB, everything else 4 MiB (MessageReader.swift:86).
    pub const DEVICE: Caps = Caps {
        frame: MAX_FRAME_PAYLOAD,
        other: MAX_OTHER_HOST_PAYLOAD,
    };
    /// A host reading a device: any message 1 MiB (StreamServer.swift:1459-1463).
    pub const HOST: Caps = Caps {
        frame: MAX_CLIENT_PAYLOAD,
        other: MAX_CLIENT_PAYLOAD,
    };
    /// A host reading a pairing connection's one kind 19: at most 4096 bytes (Door.swift:392; the
    /// host also refuses a zero length there, which a caller checks on the message).
    pub const PAIRING: Caps = Caps {
        frame: MAX_PAIRING_PAYLOAD,
        other: MAX_PAIRING_PAYLOAD,
    };

    pub fn cap(&self, kind: MessageKind) -> usize {
        if kind == MessageKind::Frame {
            self.frame
        } else {
            self.other
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReadError {
    /// A header announced more than the caps allow; nothing of its payload was read.
    TooBig(Header),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::TooBig(h) => write!(
                f,
                "kind {} announced {} bytes, over the cap",
                h.kind.as_u8(),
                h.payload_length
            ),
        }
    }
}
impl std::error::Error for ReadError {}

/// Incremental reader (bytes in, messages out) with the rules of `MessageReader`
/// (iOSClient/MessageReader.swift:6-30):
/// * header, then payload; a zero-length payload is a legal message delivered at once;
/// * a header over the caps ends the reading (`TooBig`), sticky, before any payload byte is kept;
/// * a message is delivered only whole and in order; a message cut short by the end of the
///   connection is never delivered ([`MessageReader::mid_message`] tells the caller that the end
///   came in the middle of one);
/// * unknown kinds are delivered as `MessageKind::Unknown` (the caller skips them).
///
/// Liveness is the caller's: every `push` with bytes is a sign of life (MessageReader's `onBytes`).
#[derive(Debug)]
pub struct MessageReader {
    caps: Caps,
    buf: Vec<u8>,
    pos: usize,
    header: Option<Header>,
    failed: Option<ReadError>,
}

impl MessageReader {
    pub fn new(caps: Caps) -> Self {
        MessageReader {
            caps,
            buf: Vec::new(),
            pos: 0,
            header: None,
            failed: None,
        }
    }

    /// Hands in bytes read from the connection. After a failure the bytes are dropped.
    pub fn push(&mut self, data: &[u8]) {
        if self.failed.is_some() {
            return;
        }
        if self.pos > 0 && self.pos >= self.buf.len() / 2 {
            self.buf.drain(..self.pos);
            self.pos = 0;
        }
        self.buf.extend_from_slice(data);
    }

    /// The next complete message, `Ok(None)` when more bytes are needed.
    pub fn next_message(&mut self) -> Result<Option<Message>, ReadError> {
        if let Some(e) = self.failed {
            return Err(e);
        }
        let header = match self.header {
            Some(h) => h,
            None => {
                let avail = &self.buf[self.pos..];
                let Some(h) = Header::parse(avail) else {
                    return Ok(None);
                };
                if h.payload_length > self.caps.cap(h.kind) {
                    self.failed = Some(ReadError::TooBig(h));
                    self.buf = Vec::new();
                    self.pos = 0;
                    return Err(ReadError::TooBig(h));
                }
                self.pos += HEADER_LEN;
                self.header = Some(h);
                h
            }
        };
        if self.buf.len() - self.pos < header.payload_length {
            return Ok(None);
        }
        let payload = self.buf[self.pos..self.pos + header.payload_length].to_vec();
        self.pos += header.payload_length;
        self.header = None;
        Ok(Some(Message {
            kind: header.kind,
            timestamp: header.timestamp,
            is_keyframe: header.is_keyframe,
            payload,
        }))
    }

    /// Convenience: push, then drain every complete message.
    pub fn feed(&mut self, data: &[u8]) -> Result<Vec<Message>, ReadError> {
        self.push(data);
        let mut out = Vec::new();
        while let Some(m) = self.next_message()? {
            out.push(m);
        }
        Ok(out)
    }

    /// True when the connection ending now would cut a message short (a header or part of a
    /// payload is buffered). Such a message is dropped, never delivered.
    pub fn mid_message(&self) -> bool {
        self.failed.is_none() && (self.header.is_some() || self.buf.len() > self.pos)
    }

    pub fn failure(&self) -> Option<ReadError> {
        self.failed
    }
}

/// `ParameterSets` (StreamMessage.swift:154-191): HEVC VPS/SPS/PPS raw NAL units without start
/// codes, plus the NAL length-prefix size (normally 4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParameterSets {
    pub nal_unit_header_length: u8,
    pub sets: Vec<Vec<u8>>,
}

impl ParameterSets {
    /// `encoded()`: `nalLen(1) count(1)` then per set `len(2 BE) bytes`. `None` when there are
    /// more than 255 sets or a set over 65535 bytes (Swift would trap).
    pub fn encode(&self) -> Option<Vec<u8>> {
        let count = u8::try_from(self.sets.len()).ok()?;
        let mut d = vec![self.nal_unit_header_length, count];
        for s in &self.sets {
            d.extend_from_slice(&u16::try_from(s.len()).ok()?.to_be_bytes());
            d.extend_from_slice(s);
        }
        Some(d)
    }

    /// `init?(encoded:)`: `None` when cut short; bytes after the last set are ignored (as Swift).
    pub fn decode(d: &[u8]) -> Option<ParameterSets> {
        if d.len() < 2 {
            return None;
        }
        let count = d[1] as usize;
        let mut offset = 2;
        let mut sets = Vec::with_capacity(count);
        for _ in 0..count {
            if d.len() < offset + 2 {
                return None;
            }
            let len = u16::from_be_bytes([d[offset], d[offset + 1]]) as usize;
            offset += 2;
            if d.len() < offset + len {
                return None;
            }
            sets.push(d[offset..offset + len].to_vec());
            offset += len;
        }
        Some(ParameterSets {
            nal_unit_header_length: d[0],
            sets,
        })
    }

    /// The HEVC NAL type of a set (`(b0 >> 1) & 0x3f`): 32 VPS, 33 SPS, 34 PPS.
    pub fn nal_type(set: &[u8]) -> Option<u8> {
        set.first().map(|b| (b >> 1) & 0x3f)
    }
}
