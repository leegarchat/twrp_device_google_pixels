//! Typed, zero-copy codec for the Trusty Storage Proxy protocol.
//!
//! Replaces the C `struct storage_msg { ...; uint8_t payload[0]; }` flexible
//! array idiom with explicit little-endian header parsing over `&[u8]` slices.

// Protocol registry: not every constant is used on every build, keep them all.
#![allow(dead_code)]

/// Trusty IPC port served by this proxy.
pub const STORAGE_DISK_PROXY_PORT: &str = "com.android.trusty.storage.proxy";

/// Size of the fixed message header in bytes
/// (cmd, op_id, flags, size: u32 LE + result: i32 LE + reserved: u32 LE).
pub const HEADER_SIZE: usize = 24;

// --- Command codes (request; response = request | RESP_BIT) ---
pub const STORAGE_RESP_BIT: u32 = 1;
pub const STORAGE_RESP_MSG_ERR: u32 = 1;
pub const STORAGE_FILE_DELETE: u32 = 2;
pub const STORAGE_FILE_OPEN: u32 = 4;
pub const STORAGE_FILE_CLOSE: u32 = 6;
pub const STORAGE_FILE_READ: u32 = 8;
pub const STORAGE_FILE_WRITE: u32 = 10;
pub const STORAGE_FILE_GET_SIZE: u32 = 12;
pub const STORAGE_FILE_SET_SIZE: u32 = 14;
pub const STORAGE_RPMB_SEND: u32 = 16;
pub const STORAGE_END_TRANSACTION: u32 = 18;
pub const STORAGE_FILE_GET_MAX_SIZE: u32 = 24;

// --- Error codes ---
pub const STORAGE_NO_ERROR: i32 = 0;
pub const STORAGE_ERR_GENERIC: i32 = 1;
pub const STORAGE_ERR_NOT_VALID: i32 = 2;
pub const STORAGE_ERR_UNIMPLEMENTED: i32 = 3;
pub const STORAGE_ERR_ACCESS: i32 = 4;
pub const STORAGE_ERR_NOT_FOUND: i32 = 5;
pub const STORAGE_ERR_EXIST: i32 = 6;
pub const STORAGE_ERR_TRANSACT: i32 = 7;
pub const STORAGE_ERR_SYNC_FAILURE: i32 = 8;

// --- Message flags ---
pub const STORAGE_MSG_FLAG_BATCH: u32 = 0x1;
pub const STORAGE_MSG_FLAG_PRE_COMMIT: u32 = 0x2;
pub const STORAGE_MSG_FLAG_POST_COMMIT: u32 = 0x4;
pub const STORAGE_MSG_FLAG_TRANSACT_COMPLETE: u32 = 0x4;
pub const STORAGE_MSG_FLAG_PRE_COMMIT_CHECKPOINT: u32 = 0x8;

// --- File open flags ---
pub const STORAGE_FILE_OPEN_CREATE: u32 = 1 << 0;
pub const STORAGE_FILE_OPEN_CREATE_EXCLUSIVE: u32 = 1 << 1;
pub const STORAGE_FILE_OPEN_TRUNCATE: u32 = 1 << 2;

/// Decoded message header. `size` is the total message size (header + payload).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub cmd: u32,
    pub op_id: u32,
    pub flags: u32,
    pub size: u32,
    pub result: i32,
}

/// Protocol-level parse error (malformed header or truncated frame).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    Truncated,
    BadSize,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Truncated => write!(f, "message shorter than header"),
            ParseError::BadSize => write!(f, "message size field mismatch"),
        }
    }
}

impl std::error::Error for ParseError {}

fn get_u32_le(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

/// Parses and validates a header from a received frame.
///
/// `frame` must contain exactly one full message: `frame.len()` has to equal
/// the `size` field carried in the header.
pub fn parse_frame(frame: &[u8]) -> Result<(Header, &[u8]), ParseError> {
    if frame.len() < HEADER_SIZE {
        return Err(ParseError::Truncated);
    }
    let header = Header {
        cmd: get_u32_le(frame, 0),
        op_id: get_u32_le(frame, 4),
        flags: get_u32_le(frame, 8),
        size: get_u32_le(frame, 12),
        result: get_u32_le(frame, 16) as i32,
    };
    if header.size as usize != frame.len() || header.size < HEADER_SIZE as u32 {
        return Err(ParseError::BadSize);
    }
    Ok((header, &frame[HEADER_SIZE..]))
}

/// Serializes a response header in front of `payload` into `out`.
pub fn encode_response(header: &Header, result: i32, payload: &[u8], out: &mut Vec<u8>) {
    out.clear();
    out.reserve(HEADER_SIZE + payload.len());
    out.extend_from_slice(&(header.cmd | STORAGE_RESP_BIT).to_le_bytes());
    out.extend_from_slice(&header.op_id.to_le_bytes());
    out.extend_from_slice(&header.flags.to_le_bytes());
    out.extend_from_slice(&(HEADER_SIZE as u32 + payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&result.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // __reserved
    out.extend_from_slice(payload);
}

/// Splits a NUL-terminated file name out of a request payload of the form
/// `u32 flags + name + '\0'`, validating that the name occupies the whole
/// remainder of the payload.
pub fn parse_open_like(payload: &[u8]) -> Result<(u32, &str), ()> {
    if payload.len() < 4 {
        return Err(());
    }
    let flags = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let name_bytes = &payload[4..];
    let nul = name_bytes.iter().position(|&b| b == 0).ok_or(())?;
    if nul != name_bytes.len() - 1 {
        // Name must be exactly NUL-terminated with no trailing bytes.
        return Err(());
    }
    let name = std::str::from_utf8(&name_bytes[..nul]).map_err(|_| ())?;
    if name.is_empty() {
        return Err(());
    }
    Ok((flags, name))
}

/// RPMB_SEND request: three u32 LE sizes + reliable_write + write payload.
#[derive(Debug, Clone, Copy)]
pub struct RpmbReq {
    pub reliable_write_size: u32,
    pub write_size: u32,
    pub read_size: u32,
}

pub fn parse_rpmb_req(payload: &[u8]) -> Result<(RpmbReq, &[u8]), ()> {
    if payload.len() < 16 {
        return Err(());
    }
    let req = RpmbReq {
        reliable_write_size: u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]),
        write_size: u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]),
        read_size: u32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]),
    };
    let data = &payload[16..];
    let expected = req.reliable_write_size as usize + req.write_size as usize;
    if data.len() != expected {
        return Err(());
    }
    Ok((req, data))
}

/// FILE_READ request: u32 handle + u32 size + u64 offset.
#[derive(Debug, Clone, Copy)]
pub struct ReadReq {
    pub handle: u32,
    pub size: u32,
    pub offset: u64,
}

pub fn parse_read_req(payload: &[u8]) -> Result<ReadReq, ()> {
    if payload.len() != 16 {
        return Err(());
    }
    Ok(ReadReq {
        handle: u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]),
        size: u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]),
        offset: u64::from_le_bytes([
            payload[8], payload[9], payload[10], payload[11], payload[12], payload[13],
            payload[14], payload[15],
        ]),
    })
}

/// FILE_WRITE request: u64 offset + u32 handle + u32 reserved + data.
#[derive(Debug, Clone, Copy)]
pub struct WriteReq {
    pub handle: u32,
    pub offset: u64,
}

pub fn parse_write_req(payload: &[u8]) -> Result<(WriteReq, &[u8]), ()> {
    if payload.len() < 16 {
        return Err(());
    }
    let req = WriteReq {
        offset: u64::from_le_bytes([
            payload[0], payload[1], payload[2], payload[3], payload[4], payload[5],
            payload[6], payload[7],
        ]),
        handle: u32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]),
    };
    Ok((req, &payload[16..]))
}

/// Fixed 4-byte handle request (CLOSE / GET_SIZE / GET_MAX_SIZE).
pub fn parse_handle_req(payload: &[u8]) -> Result<u32, ()> {
    if payload.len() != 4 {
        return Err(());
    }
    Ok(u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]))
}

/// FILE_SET_SIZE request: u64 size + u32 handle.
#[derive(Debug, Clone, Copy)]
pub struct SetSizeReq {
    pub handle: u32,
    pub size: u64,
}

pub fn parse_set_size_req(payload: &[u8]) -> Result<SetSizeReq, ()> {
    if payload.len() != 12 {
        return Err(());
    }
    Ok(SetSizeReq {
        size: u64::from_le_bytes([
            payload[0], payload[1], payload[2], payload[3], payload[4], payload[5],
            payload[6], payload[7],
        ]),
        handle: u32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]),
    })
}
