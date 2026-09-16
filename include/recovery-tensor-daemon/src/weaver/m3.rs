//! Titan M3 (malibu) raw-struct fallback for the GSC Weaver applet.
//!
//! Titan M3 answers the same command ids ([`super::proto`] `CMD_*`) with raw
//! little-endian structs instead of protobuf — every message is prefixed
//! with a fixed header word — so the protobuf codec fails on it with
//! `UnsupportedWireType`. Layouts (from the Titan M3 GSC weaver app):
//!   getConfig reply: `[hdr][slots][keySize][valueSize]` (u32 LE).
//!   read request:    `[hdr][slot][key..]`.
//!   read reply:      `[hdr][error][throttle][rsvd][value..]`.
//!   write request:   `[hdr][slot][key..][value..]`.
//!
//! [`super::service`] tries protobuf first and falls back here, latching M3
//! mode for the process lifetime. The legacy Titan M protobuf path is
//! untouched and stays the default.

/// Fixed header word prefixing every M3 weaver message.
pub const WEAVER_MSG_HDR: u32 = 0x000e0000;

/// Minimum getConfig reply: `[hdr][slots][keySize][valueSize]`.
const GET_CONFIG_MIN: usize = 16;
/// Fixed prefix of a read reply: `[hdr][error][throttle][rsvd]`.
const READ_REPLY_FIXED: usize = 16;

fn rd32(buf: &[u8], off: usize) -> Option<u32> {
    let bytes: [u8; 4] = buf.get(off..off + 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

/// Parsed M3 getConfig reply. Zero sizes fall back to 16: the M3 applet
/// reports keySize 0 in recovery (mirrors the vold-side keysize fallback).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct M3Geometry {
    pub slots: u32,
    pub key_size: usize,
    pub value_size: usize,
}

pub fn parse_get_config(buf: &[u8]) -> Option<M3Geometry> {
    if buf.len() < GET_CONFIG_MIN {
        return None;
    }
    if rd32(buf, 0)? != WEAVER_MSG_HDR {
        return None;
    }
    let nonzero = |v: u32| if v == 0 { 16 } else { v as usize };
    Some(M3Geometry {
        slots: rd32(buf, 4)?,
        key_size: nonzero(rd32(buf, 8)?),
        value_size: nonzero(rd32(buf, 12)?),
    })
}

/// Builds an M3 read request: `[hdr][slot][key..]`.
pub fn build_read_request(slot: u32, key: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + key.len());
    out.extend_from_slice(&WEAVER_MSG_HDR.to_le_bytes());
    out.extend_from_slice(&slot.to_le_bytes());
    out.extend_from_slice(key);
    out
}

/// Parses an M3 read reply, returning `(error, throttle_ms, value)`.
/// `value_size` comes from the latched geometry.
pub fn parse_read_response(buf: &[u8], value_size: usize) -> Option<(u32, u32, Vec<u8>)> {
    if buf.len() < READ_REPLY_FIXED + value_size {
        return None;
    }
    if rd32(buf, 0)? != WEAVER_MSG_HDR {
        return None;
    }
    let error = rd32(buf, 4)?;
    let throttle = rd32(buf, 8)?;
    let value = buf[READ_REPLY_FIXED..READ_REPLY_FIXED + value_size].to_vec();
    Some((error, throttle, value))
}

/// Builds an M3 write request: `[hdr][slot][key..][value..]`.
/// Recovery never enrolls credentials; present for completeness.
pub fn build_write_request(slot: u32, key: &[u8], value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + key.len() + value.len());
    out.extend_from_slice(&WEAVER_MSG_HDR.to_le_bytes());
    out.extend_from_slice(&slot.to_le_bytes());
    out.extend_from_slice(key);
    out.extend_from_slice(value);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le(v: u32) -> [u8; 4] {
        v.to_le_bytes()
    }

    #[test]
    fn get_config_parses_and_falls_back_on_zero_sizes() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&le(WEAVER_MSG_HDR));
        buf.extend_from_slice(&le(10)); // slots
        buf.extend_from_slice(&le(0)); // keySize 0 -> 16
        buf.extend_from_slice(&le(0)); // valueSize 0 -> 16
        let g = parse_get_config(&buf).expect("must parse");
        assert_eq!((g.slots, g.key_size, g.value_size), (10, 16, 16));
    }

    #[test]
    fn get_config_rejects_bad_header_and_short_buf() {
        assert!(parse_get_config(&[0u8; 16]).is_none());
        assert!(parse_get_config(&[0u8; 15]).is_none());
        // Protobuf bytes must not latch M3: first tag (field 0, wire 0)
        // never equals the header word.
        assert!(parse_get_config(&[0x08, 0x0a]).is_none());
    }

    #[test]
    fn read_roundtrip() {
        let key = [0xABu8; 16];
        let req = build_read_request(1, &key);
        assert_eq!(req.len(), 24);
        assert_eq!(&req[..4], &le(WEAVER_MSG_HDR));
        assert_eq!(&req[4..8], &le(1));
        assert_eq!(&req[8..], &key);

        let mut reply = Vec::new();
        reply.extend_from_slice(&le(WEAVER_MSG_HDR));
        reply.extend_from_slice(&le(0)); // error
        reply.extend_from_slice(&le(250)); // throttle
        reply.extend_from_slice(&le(0)); // rsvd
        reply.extend_from_slice(&[0xCDu8; 16]);
        let (error, throttle, value) =
            parse_read_response(&reply, 16).expect("must parse");
        assert_eq!((error, throttle), (0, 250));
        assert_eq!(value, vec![0xCDu8; 16]);
    }

    #[test]
    fn write_layout_matches_reference() {
        // Reference: 40-byte request for 16/16 key/value.
        let req = build_write_request(2, &[1u8; 16], &[2u8; 16]);
        assert_eq!(req.len(), 40);
        assert_eq!(&req[..4], &le(WEAVER_MSG_HDR));
        assert_eq!(&req[4..8], &le(2));
        assert_eq!(&req[8..24], &[1u8; 16]);
        assert_eq!(&req[24..40], &[2u8; 16]);
    }
}
