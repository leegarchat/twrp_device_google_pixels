//! Trusty secure-storage proxy daemon.
//!
//! Bridges `com.android.trusty.storage.proxy` (Trusty TIPC) with the
//! protected file store (`-p` root) and the UFS RPMB well-known LUN.
//! Reconnects to Trusty in a tight loop; on the first successful connect a
//! `/dev/.recovery_sp_ready` marker is created for init coordination.

mod fs;
mod protocol;
mod rpmb;
mod tipc;

use std::collections::HashSet;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::PathBuf;
use std::time::Duration;

use protocol::{
    Header, STORAGE_ERR_GENERIC, STORAGE_ERR_NOT_VALID, STORAGE_ERR_UNIMPLEMENTED,
    STORAGE_FILE_CLOSE, STORAGE_FILE_DELETE, STORAGE_FILE_GET_MAX_SIZE, STORAGE_FILE_GET_SIZE,
    STORAGE_FILE_OPEN, STORAGE_FILE_OPEN_CREATE, STORAGE_FILE_OPEN_CREATE_EXCLUSIVE,
    STORAGE_FILE_OPEN_TRUNCATE, STORAGE_FILE_READ, STORAGE_FILE_SET_SIZE, STORAGE_FILE_WRITE,
    STORAGE_MSG_FLAG_POST_COMMIT, STORAGE_MSG_FLAG_PRE_COMMIT, STORAGE_NO_ERROR, STORAGE_RPMB_SEND,
    STORAGE_DISK_PROXY_PORT,
};
use rpmb::Rpmb;
use tipc::{TipcConn, MAX_PAYLOAD};

/// Upper bound for a single FILE_READ response payload.
const MAX_READ_SIZE: usize = 4096;
/// Virtual max file size reported for non-block handles (as in the C proxy).
const MAX_FILE_SIZE: u64 = 0x10000000000;
/// RPMB transfer granularity.
const MMC_BLOCK_SIZE: u32 = 512;
/// Thumbstone signalling the first successful Trusty connect.
const READY_MARKER: &str = "/dev/.recovery_sp_ready";

/// Fatal (non-recoverable) setup failure. The proxy loop itself never
/// returns: TIPC disconnects are handled by reconnecting.
#[derive(Debug)]
pub struct Fatal(pub String);

impl std::fmt::Display for Fatal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for Fatal {}

struct Daemon {
    root: PathBuf,
    rpmb: Rpmb,
    /// Handles handed out to Trusty that are currently open. Guards against
    /// double-close and use of foreign fds (e.g. 0/1/2): only tracked handles
    /// are accepted by the file subcommands.
    open: HashSet<u32>,
}

/// Runs the storage proxy. Diverges (only `Err` on fatal setup failure).
pub fn run(trusty_dev: &str, rpmb_dev: Option<&str>, data_path: &str) -> Result<(), Fatal> {
    let root = PathBuf::from(data_path);
    fs::ensure_root(&root).map_err(|e| Fatal(format!("cannot prepare {data_path}: {e}")))?;

    let rpmb_path = rpmb::resolve_device(rpmb_dev).map_err(Fatal)?;
    let rpmb = Rpmb::open(&rpmb_path)
        .map_err(|e| Fatal(format!("cannot open RPMB device {rpmb_path}: {e}")))?;

    crate::logi!("sp", "starting: trusty={trusty_dev} rpmb={rpmb_path} data={data_path}");

    let mut daemon = Daemon { root, rpmb, open: HashSet::new() };
    let mut failures: u32 = 0;
    let mut first_connect = true;
    loop {
        match TipcConn::connect(trusty_dev, STORAGE_DISK_PROXY_PORT) {
            Ok(conn) => {
                failures = 0;
                crate::logd!("sp", "connected to Trusty storage");
                if first_connect {
                    signal_ready();
                    first_connect = false;
                }
                // Returns only when the connection breaks.
                if let Err(e) = daemon.proxy_loop(&conn) {
                    crate::logd!("sp", "proxy loop exited: {e}");
                }
            }
            Err(e) => {
                failures += 1;
                if failures > 100 {
                    crate::loge!("sp", "too many connect failures ({e}), sleeping 1s");
                    std::thread::sleep(Duration::from_secs(1));
                    failures = 0;
                } else {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
    }
}

fn signal_ready() {
    match std::fs::File::create(READY_MARKER) {
        Ok(_) => crate::logi!("sp", "ready (signaled {READY_MARKER})"),
        Err(e) => crate::loge!("sp", "cannot signal {READY_MARKER}: {e}"),
    }
}

impl Daemon {
    /// Serves one Trusty connection until it breaks.
    fn proxy_loop(&mut self, conn: &TipcConn) -> Result<(), tipc::ReadError> {
        let mut payload = vec![0u8; MAX_PAYLOAD + 1];
        loop {
            let (header, len) = conn.read_msg(&mut payload)?;
            let body = payload[..len].to_vec();
            let (result, reply) = self.handle(&header, &body);
            if let Err(e) = conn.respond(&header, result, &reply) {
                crate::loge!("sp", "respond failed: {e}");
                return Ok(());
            }
        }
    }

    fn handle(&mut self, header: &Header, body: &[u8]) -> (i32, Vec<u8>) {
        if header.flags & STORAGE_MSG_FLAG_PRE_COMMIT != 0 {
            // Safety: sync() has no preconditions.
            unsafe { libc::sync() };
        }
        match header.cmd {
            STORAGE_RPMB_SEND => self.rpmb_send(body),
            STORAGE_FILE_DELETE => self.file_delete(body),
            STORAGE_FILE_OPEN => self.file_open(body),
            STORAGE_FILE_CLOSE => self.file_close(body),
            STORAGE_FILE_READ => self.file_read(body),
            STORAGE_FILE_WRITE => self.file_write(header, body),
            STORAGE_FILE_GET_SIZE => self.file_get_size(body),
            STORAGE_FILE_SET_SIZE => self.file_set_size(body),
            STORAGE_FILE_GET_MAX_SIZE => self.file_get_max_size(body),
            cmd => {
                crate::loge!("sp", "unhandled command 0x{cmd:x}");
                (STORAGE_ERR_UNIMPLEMENTED, Vec::new())
            }
        }
    }

    fn rpmb_send(&self, body: &[u8]) -> (i32, Vec<u8>) {
        let (req, data) = match protocol::parse_rpmb_req(body) {
            Ok(v) => v,
            Err(()) => return (STORAGE_ERR_NOT_VALID, Vec::new()),
        };
        if req.reliable_write_size % MMC_BLOCK_SIZE != 0
            || req.write_size % MMC_BLOCK_SIZE != 0
            || req.read_size % MMC_BLOCK_SIZE != 0
            || (req.read_size as usize) > MAX_PAYLOAD
        {
            return (STORAGE_ERR_NOT_VALID, Vec::new());
        }
        let rel = req.reliable_write_size as usize;
        let (reliable, write) = data.split_at(rel);
        match self.rpmb.transact(reliable, write, req.read_size as usize) {
            Ok(out) => (STORAGE_NO_ERROR, out),
            Err(e) => {
                crate::loge!("sp", "rpmb transaction failed: {e}");
                (STORAGE_ERR_GENERIC, Vec::new())
            }
        }
    }

    fn file_delete(&self, body: &[u8]) -> (i32, Vec<u8>) {
        let (_, name) = match protocol::parse_open_like(body) {
            Ok(v) => v,
            Err(()) => return (STORAGE_ERR_NOT_VALID, Vec::new()),
        };
        let path = match fs::join_root(&self.root, name) {
            Ok(p) => p,
            Err(code) => return (code, Vec::new()),
        };
        match std::fs::remove_file(&path) {
            Ok(()) => (STORAGE_NO_ERROR, Vec::new()),
            Err(e) => (fs::translate_errno(&e), Vec::new()),
        }
    }

    fn file_open(&mut self, body: &[u8]) -> (i32, Vec<u8>) {
        let (flags, name) = match protocol::parse_open_like(body) {
            Ok(v) => v,
            Err(()) => return (STORAGE_ERR_NOT_VALID, Vec::new()),
        };
        let path = match fs::join_root(&self.root, name) {
            Ok(p) => p,
            Err(code) => return (code, Vec::new()),
        };
        let mut open_flags = libc::O_RDWR;
        if flags & STORAGE_FILE_OPEN_TRUNCATE != 0 {
            open_flags |= libc::O_TRUNC;
        }
        let creating = flags & STORAGE_FILE_OPEN_CREATE != 0;
        let exclusive = flags & STORAGE_FILE_OPEN_CREATE_EXCLUSIVE != 0;

        let fd: io::Result<OwnedFd> = if creating {
            if let Err(e) = fs::ensure_parent_dirs(&path) {
                return (fs::translate_errno(&e), Vec::new());
            }
            if exclusive {
                fs::raw_open(&path, open_flags | libc::O_CREAT | libc::O_EXCL)
            } else {
                // Open-or-create without truncation races: try plain open
                // first so existing files keep their content/attrs.
                match fs::raw_open(&path, open_flags) {
                    Ok(fd) => Ok(fd),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {
                        fs::raw_open(&path, open_flags | libc::O_CREAT)
                    }
                    Err(e) => Err(e),
                }
            }
        } else {
            fs::raw_open(&path, open_flags)
        };

        match fd {
            Ok(owned) => {
                if creating {
                    fs::sync_parent(&path);
                }
                // The raw fd number doubles as the Trusty file handle.
                let handle = owned.as_raw_fd() as u32;
                std::mem::forget(owned);
                self.open.insert(handle);
                (STORAGE_NO_ERROR, handle.to_le_bytes().to_vec())
            }
            Err(e) => (fs::translate_errno(&e), Vec::new()),
        }
    }

    /// Validates that `handle` names a currently open file.
    fn check_open(&self, handle: u32) -> Result<libc::c_int, i32> {
        if (handle as i32) < 0 || !self.open.contains(&handle) {
            return Err(STORAGE_ERR_NOT_VALID);
        }
        Ok(handle as libc::c_int)
    }

    fn file_close(&mut self, body: &[u8]) -> (i32, Vec<u8>) {
        let handle = match protocol::parse_handle_req(body) {
            Ok(h) => h,
            Err(()) => return (STORAGE_ERR_NOT_VALID, Vec::new()),
        };
        let fd = match self.check_open(handle) {
            Ok(fd) => fd,
            Err(code) => return (code, Vec::new()),
        };
        // Safety: fd comes from the validated open-handle table.
        unsafe { libc::fsync(fd) };
        // Safety: fd comes from the validated open-handle table; removed right after.
        let rc = unsafe { libc::close(fd) };
        self.open.remove(&handle);
        if rc < 0 {
            (fs::translate_errno(&io::Error::last_os_error()), Vec::new())
        } else {
            (STORAGE_NO_ERROR, Vec::new())
        }
    }

    fn file_read(&self, body: &[u8]) -> (i32, Vec<u8>) {
        let req = match protocol::parse_read_req(body) {
            Ok(r) => r,
            Err(()) => return (STORAGE_ERR_NOT_VALID, Vec::new()),
        };
        if req.size as usize > MAX_READ_SIZE {
            return (STORAGE_ERR_NOT_VALID, Vec::new());
        }
        let fd = match self.check_open(req.handle) {
            Ok(fd) => fd,
            Err(code) => return (code, Vec::new()),
        };
        let mut buf = vec![0u8; req.size as usize];
        match fs::read_at(fd, &mut buf, req.offset) {
            Ok(n) => {
                buf.truncate(n);
                (STORAGE_NO_ERROR, buf)
            }
            Err(e) => (fs::translate_errno(&e), Vec::new()),
        }
    }

    fn file_write(&self, header: &Header, body: &[u8]) -> (i32, Vec<u8>) {
        let (req, data) = match protocol::parse_write_req(body) {
            Ok(v) => v,
            Err(()) => return (STORAGE_ERR_NOT_VALID, Vec::new()),
        };
        let fd = match self.check_open(req.handle) {
            Ok(fd) => fd,
            Err(code) => return (code, Vec::new()),
        };
        if let Err(e) = fs::write_at(fd, data, req.offset) {
            return (fs::translate_errno(&e), Vec::new());
        }
        if header.flags & STORAGE_MSG_FLAG_POST_COMMIT != 0 {
            // The C version had a phantom empty fd-table loop here; a plain
            // sync() is the intended barrier.
            // Safety: sync() has no preconditions.
            unsafe { libc::sync() };
        }
        (STORAGE_NO_ERROR, Vec::new())
    }

    fn file_get_size(&self, body: &[u8]) -> (i32, Vec<u8>) {
        let handle = match protocol::parse_handle_req(body) {
            Ok(h) => h,
            Err(()) => return (STORAGE_ERR_NOT_VALID, Vec::new()),
        };
        let fd = match self.check_open(handle) {
            Ok(fd) => fd,
            Err(code) => return (code, Vec::new()),
        };
        // Safety: all-zero bit pattern is valid for plain-old-data libc::stat.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        // Safety: fd is valid; &mut st points at a live stat struct.
            if unsafe { libc::fstat(fd, &mut st) } < 0 {
            return (fs::translate_errno(&io::Error::last_os_error()), Vec::new());
        }
        (STORAGE_NO_ERROR, (st.st_size as u64).to_le_bytes().to_vec())
    }

    fn file_set_size(&self, body: &[u8]) -> (i32, Vec<u8>) {
        let req = match protocol::parse_set_size_req(body) {
            Ok(r) => r,
            Err(()) => return (STORAGE_ERR_NOT_VALID, Vec::new()),
        };
        let fd = match self.check_open(req.handle) {
            Ok(fd) => fd,
            Err(code) => return (code, Vec::new()),
        };
        // Safety: fd comes from the validated open-handle table; length is protocol-bounded.
            if unsafe { libc::ftruncate(fd, req.size as libc::off_t) } < 0 {
            return (fs::translate_errno(&io::Error::last_os_error()), Vec::new());
        }
        (STORAGE_NO_ERROR, Vec::new())
    }

    fn file_get_max_size(&self, body: &[u8]) -> (i32, Vec<u8>) {
        let handle = match protocol::parse_handle_req(body) {
            Ok(h) => h,
            Err(()) => return (STORAGE_ERR_NOT_VALID, Vec::new()),
        };
        let fd = match self.check_open(handle) {
            Ok(fd) => fd,
            Err(code) => return (code, Vec::new()),
        };
        // Safety: all-zero bit pattern is valid for plain-old-data libc::stat.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        // Safety: fd is valid; &mut st points at a live stat struct.
            if unsafe { libc::fstat(fd, &mut st) } < 0 {
            return (fs::translate_errno(&io::Error::last_os_error()), Vec::new());
        }
        let max: u64 = if ((st.st_mode as libc::mode_t) & libc::S_IFMT) == libc::S_IFBLK {
            const BLKGETSIZE64: libc::c_ulong = 0x80081272;
            let mut size: u64 = 0;
            // Safety: fd is a valid block-device fd; BLKGETSIZE64 fits i32; &mut size is valid.
            if unsafe { libc::ioctl(fd, BLKGETSIZE64 as _, &mut size) } < 0 {
                return (fs::translate_errno(&io::Error::last_os_error()), Vec::new());
            }
            size
        } else {
            MAX_FILE_SIZE
        };
        (STORAGE_NO_ERROR, max.to_le_bytes().to_vec())
    }
}
