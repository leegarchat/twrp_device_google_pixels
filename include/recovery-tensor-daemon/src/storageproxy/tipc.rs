//! Trusty IPC (TIPC) client: connect + message-oriented read/write.
//!
//! The Trusty device node (e.g. `/dev/trusty-ipc-dev0`) is opened read/write
//! and bound to a port with `TIPC_IOC_CONNECT`; afterwards each `readv` of
//! (header, payload) yields exactly one storage message, mirroring the
//! original C proxy.

use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use super::protocol::{self, Header, ParseError, HEADER_SIZE};

/// `_IOW('r', 0x80, char *)`: pointer-sized, so computed from the target.
const fn iow_u32(ty: u32, nr: u32, size: u32) -> u32 {
    (1 << 30) | (size << 16) | (ty << 8) | nr
}

fn tipc_ioc_connect() -> libc::c_ulong {
    iow_u32(b'r' as u32, 0x80, std::mem::size_of::<*const u8>() as u32) as libc::c_ulong
}

/// Maximum single Trusty message payload we accept.
pub const MAX_PAYLOAD: usize = 4096;

/// Opened TIPC connection. Closes the fd on drop.
pub struct TipcConn {
    fd: OwnedFd,
}

impl TipcConn {
    /// Opens `dev` and connects to `port` (NUL-terminated C string).
    pub fn connect(dev: &str, port: &str) -> io::Result<TipcConn> {
        let dev_c = CString::new(dev).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "trusty device path contains NUL")
        })?;
        let port_c = CString::new(port).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "trusty port contains NUL")
        })?;
        // Mode 0600: the fd hands out Trusty secure-storage access.
        // Safety: path is a valid CString; flags/mode are valid; fd checked below.
        let raw = unsafe { libc::open(dev_c.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC, 0o600) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `open` succeeded, we own the fd.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        // Safety: fd is valid; request code fits i32; port string is a valid CString pointer.
        let rc = unsafe { libc::ioctl(fd.as_raw_fd(), tipc_ioc_connect() as _, port_c.as_ptr()) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(TipcConn { fd })
    }

    /// Reads exactly one message: header + up to `MAX_PAYLOAD` payload bytes.
    ///
    /// Returns the decoded header and the payload length. `payload` must be
    /// at least `MAX_PAYLOAD + 1` bytes (mirrors the C `req_buffer` + NUL).
    pub fn read_msg(&self, payload: &mut [u8]) -> Result<(Header, usize), ReadError> {
        assert!(payload.len() > MAX_PAYLOAD);
        let mut hdr_buf = [0u8; HEADER_SIZE];
        // Two-element readv keeps header and payload split without copying.
        let iovs = [
            libc::iovec {
                iov_base: hdr_buf.as_mut_ptr() as *mut libc::c_void,
                iov_len: hdr_buf.len(),
            },
            libc::iovec {
                iov_base: payload.as_mut_ptr() as *mut libc::c_void,
                iov_len: MAX_PAYLOAD + 1,
            },
        ];
        let rc = loop {
            // Safety: fd is valid; iovs describes the two live stack buffers.
            let rc = unsafe { libc::readv(self.fd.as_raw_fd(), iovs.as_ptr(), 2) };
            if rc < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(ReadError::Io(e));
            }
            break rc;
        };
        if (rc as usize) < HEADER_SIZE {
            return Err(ReadError::Truncated);
        }
        // Reassemble one contiguous frame for zero-copy header validation.
        let total = rc as usize;
        let mut frame = [0u8; HEADER_SIZE + MAX_PAYLOAD + 1];
        frame[..HEADER_SIZE].copy_from_slice(&hdr_buf);
        frame[HEADER_SIZE..total].copy_from_slice(&payload[..total - HEADER_SIZE]);
        let (header, body) = protocol::parse_frame(&frame[..total]).map_err(ReadError::Parse)?;
        payload[..body.len()].copy_from_slice(body);
        Ok((header, body.len()))
    }

    /// Sends a response for `header` with `result` and `payload`.
    ///
    /// The Trusty node is message-oriented: the reply goes out as a single
    /// `writev` (header, payload); anything but a full write is an error.
    pub fn respond(&self, header: &Header, result: i32, payload: &[u8]) -> io::Result<()> {
        let mut wire = Vec::with_capacity(HEADER_SIZE + payload.len());
        protocol::encode_response(header, result, payload, &mut wire);
        let (head, body) = wire.split_at(HEADER_SIZE);
        let iovs = [
            libc::iovec {
                iov_base: head.as_ptr() as *mut libc::c_void,
                iov_len: head.len(),
            },
            libc::iovec {
                iov_base: body.as_ptr() as *mut libc::c_void,
                iov_len: body.len(),
            },
        ];
        let niov = if body.is_empty() { 1 } else { 2 };
        loop {
            // Safety: fd is valid; iovs describes live buffers; niov matches.
            let rc = unsafe { libc::writev(self.fd.as_raw_fd(), iovs.as_ptr(), niov) };
            if rc < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            if (rc as usize) != wire.len() {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "tipc short writev on message fd",
                ));
            }
            return Ok(());
        }
    }
}

/// Failure of [`TipcConn::read_msg`].
#[derive(Debug)]
pub enum ReadError {
    Io(io::Error),
    Truncated,
    Parse(ParseError),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Io(e) => write!(f, "tipc read: {e}"),
            ReadError::Truncated => write!(f, "tipc read: short frame"),
            ReadError::Parse(e) => write!(f, "tipc read: {e}"),
        }
    }
}

impl std::error::Error for ReadError {}
