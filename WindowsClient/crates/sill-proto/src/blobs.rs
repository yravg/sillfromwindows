//! Binary payloads: `ImageBlob` (Switcher.swift:190-218, kinds 3 and 4) and `CursorShapeBlob`
//! (Viewport.swift:141-158, kind 14).

/// Kind 3: `windowID(4 BE) + JPEG`. `None` unless more than 4 bytes (Switcher.swift:198).
pub fn decode_thumbnail(d: &[u8]) -> Option<(u32, &[u8])> {
    if d.len() <= 4 {
        return None;
    }
    Some((u32::from_be_bytes(d[..4].try_into().ok()?), &d[4..]))
}

pub fn encode_thumbnail(window_id: u32, jpeg: &[u8]) -> Vec<u8> {
    let mut d = Vec::with_capacity(4 + jpeg.len());
    d.extend_from_slice(&window_id.to_be_bytes());
    d.extend_from_slice(jpeg);
    d
}

/// Kind 4: `bundleIDLength(2 BE) + bundleID UTF-8 + PNG`. `None` unless the buffer is longer than
/// 2 + length (an empty PNG is refused), or the id is not UTF-8 (Switcher.swift:212-218).
pub fn decode_icon(d: &[u8]) -> Option<(String, &[u8])> {
    if d.len() <= 2 {
        return None;
    }
    let len = u16::from_be_bytes([d[0], d[1]]) as usize;
    if d.len() <= 2 + len {
        return None;
    }
    let id = std::str::from_utf8(&d[2..2 + len]).ok()?;
    Some((id.to_string(), &d[2 + len..]))
}

/// `None` when the id is over 65535 bytes (Swift would trap).
pub fn encode_icon(bundle_id: &str, png: &[u8]) -> Option<Vec<u8>> {
    let id = bundle_id.as_bytes();
    let n = u16::try_from(id.len()).ok()?;
    let mut d = Vec::with_capacity(2 + id.len() + png.len());
    d.extend_from_slice(&n.to_be_bytes());
    d.extend_from_slice(id);
    d.extend_from_slice(png);
    Some(d)
}

/// Kind 14: four Float32 BE (hotspot x, y, width, height in Mac points) then a PNG.
#[derive(Clone, Debug, PartialEq)]
pub struct CursorShape<'a> {
    pub hotspot: (f32, f32),
    pub size: (f32, f32),
    pub png: &'a [u8],
}

/// `CursorShapeBlob.decode`: needs more than 16 bytes (Viewport.swift:153).
pub fn decode_cursor_shape(d: &[u8]) -> Option<CursorShape<'_>> {
    if d.len() <= 16 {
        return None;
    }
    let f = |i: usize| f32::from_bits(u32::from_be_bytes(d[i * 4..i * 4 + 4].try_into().unwrap()));
    Some(CursorShape {
        hotspot: (f(0), f(1)),
        size: (f(2), f(3)),
        png: &d[16..],
    })
}

pub fn encode_cursor_shape(hotspot: (f32, f32), size: (f32, f32), png: &[u8]) -> Vec<u8> {
    let mut d = Vec::with_capacity(16 + png.len());
    for v in [hotspot.0, hotspot.1, size.0, size.1] {
        d.extend_from_slice(&v.to_bits().to_be_bytes());
    }
    d.extend_from_slice(png);
    d
}
