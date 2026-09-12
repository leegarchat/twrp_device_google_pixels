//! Minimal zero-copy protobuf codec for the GSC Weaver applet.
//!
//! Only what the applet speaks is implemented: varint (wire 0), fixed64
//! (wire 1), length-delimited (wire 2) and fixed32 (wire 5). Unknown fields
//! of any of these wire types are skipped correctly; anything else
//! (groups, reserved types) is a hard error instead of silently stalling.
//!
//! Field layout (matches the Citadel weaver app):
//!   GetConfig response: 1 = slots, 2 = key_size, 3 = value_size (varints).
//!   Read request:       1 = slot (varint), 2 = key (bytes).
//!   Read response:      1 = error (varint), 2 = throttle_ms (varint),
//!                       3 = value (bytes).
//!   Write request:      1 = slot (varint), 2 = key (bytes), 3 = value (bytes).

/// Weaver applet command ids (`params` of the NOS call).
pub const CMD_GET_CONFIG: u16 = 0;
pub const CMD_WRITE: u16 = 1;
pub const CMD_READ: u16 = 2;

/// Codec failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtoError {
    Truncated,
    VarintOverflow,
    UnsupportedWireType(u32),
    LengthOverflow,
}

impl std::fmt::Display for ProtoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtoError::Truncated => write!(f, "protobuf: truncated buffer"),
            ProtoError::VarintOverflow => write!(f, "protobuf: varint overflow"),
            ProtoError::UnsupportedWireType(w) => {
                write!(f, "protobuf: unsupported wire type {w}")
            }
            ProtoError::LengthOverflow => write!(f, "protobuf: length-delimited overflow"),
        }
    }
}

impl std::error::Error for ProtoError {}

// --- Encoding ---

fn encode_varint(out: &mut Vec<u8>, mut val: u64) {
    while val > 0x7F {
        out.push((val as u8 & 0x7F) | 0x80);
        val >>= 7;
    }
    out.push(val as u8);
}

fn encode_uint32_field(out: &mut Vec<u8>, field: u32, val: u32) {
    encode_varint(out, (field as u64) << 3);
    encode_varint(out, val as u64);
}

fn encode_bytes_field(out: &mut Vec<u8>, field: u32, data: &[u8]) {
    encode_varint(out, (field as u64) << 3 | 2);
    encode_varint(out, data.len() as u64);
    out.extend_from_slice(data);
}

/// Builds a Read request: `slot` + `key`.
pub fn build_read_request(slot: u32, key: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + key.len());
    encode_uint32_field(&mut out, 1, slot);
    encode_bytes_field(&mut out, 2, key);
    out
}

/// Builds a Write request: `slot` + `key` + `value`.
pub fn build_write_request(slot: u32, key: &[u8], value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + key.len() + value.len());
    encode_uint32_field(&mut out, 1, slot);
    encode_bytes_field(&mut out, 2, key);
    encode_bytes_field(&mut out, 3, value);
    out
}

// --- Decoding ---

/// Decodes one varint, advancing `pos`.
fn decode_varint(buf: &[u8], pos: &mut usize) -> Result<u64, ProtoError> {
    let mut val: u64 = 0;
    let mut shift = 0u32;
    while *pos < buf.len() {
        let b = buf[*pos];
        *pos += 1;
        val |= ((b & 0x7F) as u64) << shift;
        if b & 0x80 == 0 {
            return Ok(val);
        }
        shift += 7;
        if shift >= 64 {
            return Err(ProtoError::VarintOverflow);
        }
    }
    Err(ProtoError::Truncated)
}

/// Skips one field value of the given wire type, advancing `pos`.
fn skip_field(buf: &[u8], pos: &mut usize, wire: u32) -> Result<(), ProtoError> {
    match wire {
        0 => {
            decode_varint(buf, pos).map(|_| ())?;
        }
        1 => {
            *pos = pos.checked_add(8).ok_or(ProtoError::LengthOverflow)?;
            if *pos > buf.len() {
                return Err(ProtoError::Truncated);
            }
        }
        2 => {
            let len = decode_varint(buf, pos)?;
            let len = usize::try_from(len).map_err(|_| ProtoError::LengthOverflow)?;
            *pos = pos.checked_add(len).ok_or(ProtoError::LengthOverflow)?;
            if *pos > buf.len() {
                return Err(ProtoError::Truncated);
            }
        }
        5 => {
            *pos = pos.checked_add(4).ok_or(ProtoError::LengthOverflow)?;
            if *pos > buf.len() {
                return Err(ProtoError::Truncated);
            }
        }
        w => return Err(ProtoError::UnsupportedWireType(w)),
    }
    Ok(())
}

/// Reads one length-delimited slice, advancing `pos`.
fn take_bytes<'a>(buf: &'a [u8], pos: &mut usize) -> Result<&'a [u8], ProtoError> {
    let len = decode_varint(buf, pos)?;
    let len = usize::try_from(len).map_err(|_| ProtoError::LengthOverflow)?;
    let end = pos.checked_add(len).ok_or(ProtoError::LengthOverflow)?;
    if end > buf.len() {
        return Err(ProtoError::Truncated);
    }
    let s = &buf[*pos..end];
    *pos = end;
    Ok(s)
}

/// Parsed GetConfig response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeaverConfigWire {
    pub slots: u32,
    pub key_size: u32,
    pub value_size: u32,
}

pub fn parse_get_config(buf: &[u8]) -> Result<WeaverConfigWire, ProtoError> {
    let mut slots = 0u32;
    let mut key_size = 0u32;
    let mut value_size = 0u32;
    let mut pos = 0;
    while pos < buf.len() {
        let tag = decode_varint(buf, &mut pos)?;
        let field = (tag >> 3) as u32;
        let wire = (tag & 0x7) as u32;
        match (field, wire) {
            (1, 0) => slots = decode_varint(buf, &mut pos)? as u32,
            (2, 0) => key_size = decode_varint(buf, &mut pos)? as u32,
            (3, 0) => value_size = decode_varint(buf, &mut pos)? as u32,
            (_, _) => skip_field(buf, &mut pos, wire)?,
        }
    }
    Ok(WeaverConfigWire { slots, key_size, value_size })
}

/// Parsed Read response: `(error, throttle_ms, value)`.
pub fn parse_read_response(buf: &[u8]) -> Result<(u32, u32, Vec<u8>), ProtoError> {
    let mut error = 0u32;
    let mut throttle_ms = 0u32;
    let mut value = Vec::new();
    let mut pos = 0;
    while pos < buf.len() {
        let tag = decode_varint(buf, &mut pos)?;
        let field = (tag >> 3) as u32;
        let wire = (tag & 0x7) as u32;
        match (field, wire) {
            (1, 0) => error = decode_varint(buf, &mut pos)? as u32,
            (2, 0) => throttle_ms = decode_varint(buf, &mut pos)? as u32,
            (3, 2) => value = take_bytes(buf, &mut pos)?.to_vec(),
            (_, _) => skip_field(buf, &mut pos, wire)?,
        }
    }
    Ok((error, throttle_ms, value))
}
