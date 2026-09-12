//! Titan M (GSC/Citadel) transport via `GSC_IOC_GSA_NOS_CALL` one-pass ioctl.
//!
//! Unlike the C++ daemon — which shared one global `gsa_nos_call_buf`
//! between all Binder threads — the transfer buffer lives inside
//! [`GscDevice`] behind a mutex, so concurrent `read`/`write` calls from the
//! Binder thread pool cannot interleave.

use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::Mutex;

/// Maximum single transfer through the GSC one-pass call.
pub const MAX_GSA_NOS_CALL_TRANSFER: usize = 4096;
/// Weaver application id on the GSC NOS interface.
pub const APP_ID_WEAVER: u8 = 0x03;
/// `call_status` value meaning the applet accepted the command.
pub const APP_SUCCESS: u32 = 0;

const fn iow_u32(ty: u32, nr: u32, size: u32) -> u32 {
    (1 << 30) | (size << 16) | (ty << 8) | nr
}

/// `_IOW('c', 3, struct gsa_ioc_nos_call_req)`; the struct is 24 bytes on
/// both 32- and 64-bit (u64 `buf` field forces 8-byte alignment).
const GSC_IOC_GSA_NOS_CALL: libc::c_ulong = iow_u32(b'c' as u32, 3, 24) as libc::c_ulong;

/// Kernel one-pass call descriptor (must stay 24 bytes).
#[repr(C)]
struct GsaIocNosCallReq {
    app_id: u8,
    reserved: u8,
    params: u16,
    arg_len: u32,
    buf: u64,
    reply_len: u32,
    call_status: u32,
}

const _: () = assert!(std::mem::size_of::<GsaIocNosCallReq>() == 24);

/// Failure to talk to `/dev/gsc0`.
#[derive(Debug)]
pub enum GscError {
    Open(io::Error),
    Unsupported(&'static str),
    TooLarge { len: usize },
    Ioctl(io::Error),
}

impl std::fmt::Display for GscError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GscError::Open(e) => write!(f, "cannot open gsc device: {e}"),
            GscError::Unsupported(s) => write!(f, "gsc one_pass_call unsupported: {s}"),
            GscError::TooLarge { len } => {
                write!(f, "transfer too large: {len} > {MAX_GSA_NOS_CALL_TRANSFER}")
            }
            GscError::Ioctl(e) => write!(f, "gsc ioctl failed: {e}"),
        }
    }
}

impl std::error::Error for GscError {}

/// Opened GSC device with a call-local transfer buffer.
///
/// `Send + Sync`: the mutex serializes concurrent Binder-thread ioctls that
/// share the single kernel transfer buffer.
pub struct GscDevice {
    fd: OwnedFd,
    buf: Mutex<Vec<u8>>,
}

impl GscDevice {
    /// Opens `dev` (e.g. `/dev/gsc0`) and probes one-pass call support.
    pub fn open(dev: &str) -> Result<GscDevice, GscError> {
        let dev_c =
            CString::new(dev).map_err(|_| GscError::Open(io::Error::new(
                io::ErrorKind::InvalidInput,
                "gsc device path contains NUL",
            )))?;
        let raw = unsafe { libc::open(dev_c.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
        if raw < 0 {
            return Err(GscError::Open(io::Error::last_os_error()));
        }
        // SAFETY: owned fd from a successful open.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let this = GscDevice { fd, buf: Mutex::new(vec![0u8; MAX_GSA_NOS_CALL_TRANSFER]) };
        this.probe()?;
        crate::logi!("weaver", "opened {dev}");
        Ok(this)
    }

    /// Zero-length probe: EINVAL/ENOTTY means the kernel lacks one-pass calls.
    fn probe(&self) -> Result<(), GscError> {
        let mut guard = self.buf.lock().map_err(|_| {
            GscError::Unsupported("transfer buffer lock poisoned")
        })?;
        let mut req = GsaIocNosCallReq {
            app_id: 0,
            reserved: 0,
            params: 0,
            arg_len: 0,
            buf: guard.as_mut_ptr() as u64,
            reply_len: 0,
            call_status: 0,
        };
        let rc = unsafe { libc::ioctl(self.fd.as_raw_fd(), GSC_IOC_GSA_NOS_CALL, &mut req) };
        if rc < 0 {
            let e = io::Error::last_os_error();
            match e.raw_os_error() {
                Some(code) if code == libc::EINVAL || code == libc::ENOTTY => {
                    return Err(GscError::Unsupported("kernel lacks GSC one_pass_call"));
                }
                _ => {
                    // Any other error (e.g. applet-level rejection of app 0)
                    // still proves the ioctl itself is implemented.
                    crate::logd!("weaver", "probe ioctl returned errno {e}, continuing");
                }
            }
        }
        Ok(())
    }

    /// Issues one applet call: sends `args`, returns `(reply, call_status)`.
    pub fn nos_call(
        &self,
        app_id: u8,
        params: u16,
        args: &[u8],
        reply_cap: usize,
    ) -> Result<(Vec<u8>, u32), GscError> {
        if args.len() > MAX_GSA_NOS_CALL_TRANSFER {
            return Err(GscError::TooLarge { len: args.len() });
        }
        if reply_cap > MAX_GSA_NOS_CALL_TRANSFER {
            return Err(GscError::TooLarge { len: reply_cap });
        }
        let mut guard =
            self.buf.lock().map_err(|_| GscError::Unsupported("transfer buffer lock poisoned"))?;
        guard[..args.len()].copy_from_slice(args);
        let mut req = GsaIocNosCallReq {
            app_id,
            reserved: 0,
            params,
            arg_len: args.len() as u32,
            buf: guard.as_mut_ptr() as u64,
            reply_len: reply_cap as u32,
            call_status: 0,
        };
        let rc = unsafe { libc::ioctl(self.fd.as_raw_fd(), GSC_IOC_GSA_NOS_CALL, &mut req) };
        if rc < 0 {
            return Err(GscError::Ioctl(io::Error::last_os_error()));
        }
        let n = (req.reply_len as usize).min(guard.len());
        Ok((guard[..n].to_vec(), req.call_status))
    }
}
