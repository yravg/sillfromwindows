//! Sources/StreamProtocol/Audio.swift: kind 43, the Mac's sound. Byte 0 is the type: 1 a JSON
//! `AudioFormat`, 2 binary packets.

use crate::wire;
use serde::{Deserialize, Serialize};

/// `AudioCodec` (Audio.swift:21-40).
pub const CODEC_AAC_ELD: &str = "aac-eld";

/// `AudioCodec.choose`: the first of the device's offered codecs the host makes.
pub fn choose_codec<'a>(offered: Option<&'a [String]>, makes: &[&str]) -> Option<&'a str> {
    offered?
        .iter()
        .map(String::as_str)
        .find(|c| makes.contains(c))
}

/// `AudioFormat` (Audio.swift:43-89): type 1 JSON. `cookie` is base64 on the wire (whether it is
/// a bare AudioSpecificConfig or an ES descriptor is UNCLEAR, see the spec).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioFormat {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_rate: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames_per_packet: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priming_frames: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitrate: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<i64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "wire::b64_opt"
    )]
    pub cookie: Option<Vec<u8>>,
    /// "app", "desktop" or "test".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
}

/// `AudioPackets` (Audio.swift:91-130): type 2.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioPackets {
    pub epoch: u16,
    pub seq: u32,
    pub segment_start: bool,
    pub packets: Vec<Vec<u8>>,
}

impl AudioPackets {
    pub const MAX_COUNT: usize = 255;
    pub const MAX_PACKET_BYTES: usize = 65_535;
    pub const HEADER_BYTES: usize = 9;

    /// `serialized()`: `02, epoch u16 BE, seq u32 BE, flags, count, count x (len u16 BE, bytes)`.
    /// At most 255 packets are written, each cut to 65,535 bytes.
    pub fn serialize(&self) -> Vec<u8> {
        let kept = &self.packets[..self.packets.len().min(Self::MAX_COUNT)];
        let mut d = vec![AudioMessage::TYPE_PACKETS];
        d.extend_from_slice(&self.epoch.to_be_bytes());
        d.extend_from_slice(&self.seq.to_be_bytes());
        d.push(u8::from(self.segment_start));
        d.push(kept.len() as u8);
        for p in kept {
            let n = p.len().min(Self::MAX_PACKET_BYTES);
            d.extend_from_slice(&(n as u16).to_be_bytes());
            d.extend_from_slice(&p[..n]);
        }
        d
    }
}

/// `AudioMessage` (Audio.swift:132-192).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioMessage {
    Format(AudioFormat),
    Packets(AudioPackets),
}

impl AudioMessage {
    pub const TYPE_FORMAT: u8 = 1;
    pub const TYPE_PACKETS: u8 = 2;
    /// The reader's caps, part of the compatibility floor (whole payload, type byte included).
    pub const MAX_FORMAT_PAYLOAD: usize = 4096;
    pub const MAX_PACKETS_PAYLOAD: usize = 16384;

    pub fn serialize(&self) -> Vec<u8> {
        match self {
            AudioMessage::Format(f) => {
                let mut d = vec![Self::TYPE_FORMAT];
                d.extend_from_slice(&wire::encode(f));
                d
            }
            AudioMessage::Packets(p) => p.serialize(),
        }
    }

    /// `parse`: `None` for an empty payload, an unknown type, a format over 4 KB or not JSON, packets
    /// over 16 KB, cut short, with a count of 0, a length past the end, or bytes left over.
    pub fn parse(payload: &[u8]) -> Option<AudioMessage> {
        match *payload.first()? {
            Self::TYPE_FORMAT => {
                if payload.len() > Self::MAX_FORMAT_PAYLOAD {
                    return None;
                }
                wire::decode_object(&payload[1..]).map(AudioMessage::Format)
            }
            Self::TYPE_PACKETS => {
                if payload.len() > Self::MAX_PACKETS_PAYLOAD
                    || payload.len() < AudioPackets::HEADER_BYTES
                {
                    return None;
                }
                let epoch = u16::from_be_bytes([payload[1], payload[2]]);
                let seq = u32::from_be_bytes([payload[3], payload[4], payload[5], payload[6]]);
                let flags = payload[7];
                let count = payload[8] as usize;
                if count < 1 {
                    return None;
                }
                let mut packets = Vec::with_capacity(count);
                let mut offset = AudioPackets::HEADER_BYTES;
                for _ in 0..count {
                    if offset + 2 > payload.len() {
                        return None;
                    }
                    let len = u16::from_be_bytes([payload[offset], payload[offset + 1]]) as usize;
                    offset += 2;
                    if offset + len > payload.len() {
                        return None;
                    }
                    packets.push(payload[offset..offset + len].to_vec());
                    offset += len;
                }
                if offset != payload.len() {
                    return None;
                }
                Some(AudioMessage::Packets(AudioPackets {
                    epoch,
                    seq,
                    segment_start: flags & 1 == 1,
                    packets,
                }))
            }
            _ => None,
        }
    }
}
