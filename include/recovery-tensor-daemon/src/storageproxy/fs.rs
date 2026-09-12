//! Safe file operations for the Trusty protected store (`/tmp/ss`).
//!
//! Fixes two defects of the C proxy: multi-level file names failed because
//! only a single `mkdir` was attempted (here: `create_dir_all`), and names
//! are validated so a compromised Trusty cannot escape the data root.

use std::ffi::CString;
use std::io;
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Component, Path, PathBuf};

use super::protocol::{
    STORAGE_ERR_ACCESS, STORAGE_ERR_EXIST, STORAGE_ERR_GENERIC, STORAGE_ERR_NOT_FOUND,
    STORAGE_ERR_NOT_VALID,
};

/// Rejects absolute paths, NUL bytes and `..` escapes; returns the path
/// joined onto `root`.
pub fn join_root(root: &Path, name: &str) -> Result<PathBuf, i32> {
    if name.is_empty() || name.contains('\0') {
        return Err(STORAGE_ERR_NOT_VALID);
    }
    let rel = Path::new(name);
    if rel.is_absolute() {
        return Err(STORAGE_ERR_NOT_VALID);
    }
    for comp in rel.components() {
        match comp {
            Component::Normal(_) => {}
            // Single `.` / redundant separators are harmless; anything else
            // (ParentDir, Prefix, RootDir) is rejected.
            Component::CurDir => {}
            _ => return Err(STORAGE_ERR_NOT_VALID),
        }
    }
    Ok(root.join(rel))
}

/// Creates the data root and its `persist` child (multi-level safe).
pub fn ensure_root(root: &Path) -> io::Result<()> {
    std::fs::create_dir_all(root)?;
    // Best effort hardening; umask (077) already restricts modes.
    let _ = std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700));
    std::fs::create_dir_all(root.join("persist"))?;
    Ok(())
}

/// Creates all missing parent directories of `path` and fsyncs the parent.
pub fn ensure_parent_dirs(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
            sync_parent(path);
        }
    }
    Ok(())
}

/// fsyncs the directory containing `path` (best effort).
pub fn sync_parent(path: &Path) {
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => return,
    };
    let c = match CString::new(parent.as_os_str().as_bytes()) {
        Ok(c) => c,
        Err(_) => return,
    };
    // Safety: path is a valid CString; flags are valid; fd checked below.
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if fd >= 0 {
        // Safety: fd is valid and owned here; no use after close.
        unsafe {
            libc::fsync(fd);
            libc::close(fd);
        }
    }
}

/// Opens `path` with raw flags, returning an owned fd (the numeric value is
/// handed to Trusty as the file handle, as in the C proxy).
pub fn raw_open(path: &Path, flags: libc::c_int) -> io::Result<OwnedFd> {
    let c = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL")
    })?;
    // Safety: path is a valid CString; flags are valid; fd checked below.
    let fd = unsafe { libc::open(c.as_ptr(), flags | libc::O_CLOEXEC, 0o600) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: just opened, owned.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Full pread loop at `offset` (up to `buf.len()`; short count at EOF).
pub fn read_at(fd: libc::c_int, mut buf: &mut [u8], mut offset: u64) -> io::Result<usize> {
    let mut total = 0;
    while !buf.is_empty() {
        // Safety: fd is valid; buf is a live exclusive slice of stated length.
    let n = unsafe {
            libc::pread(
                fd,
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
                offset as libc::off_t,
            )
        };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if n == 0 {
            break;
        }
        let n = n as usize;
        buf = &mut buf[n..];
        offset += n as u64;
        total += n;
    }
    Ok(total)
}

/// Full pwrite loop at `offset`.
pub fn write_at(fd: libc::c_int, mut buf: &[u8], mut offset: u64) -> io::Result<()> {
    while !buf.is_empty() {
        // Safety: fd is valid; buf is a live shared slice of stated length.
    let n = unsafe {
            libc::pwrite(
                fd,
                buf.as_ptr() as *const libc::c_void,
                buf.len(),
                offset as libc::off_t,
            )
        };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        let n = n as usize;
        buf = &buf[n..];
        offset += n as u64;
    }
    Ok(())
}

/// Maps an OS error to a Trusty storage error code.
pub fn translate_errno(e: &io::Error) -> i32 {
    match e.raw_os_error() {
        Some(code) => match code {
            _ if code == libc::ENOENT => STORAGE_ERR_NOT_FOUND,
            _ if code == libc::EEXIST => STORAGE_ERR_EXIST,
            _ if code == libc::EACCES || code == libc::EPERM => STORAGE_ERR_ACCESS,
            _ if code == libc::EBADF
                || code == libc::EINVAL
                || code == libc::ENOTDIR
                || code == libc::ENAMETOOLONG =>
            {
                STORAGE_ERR_NOT_VALID
            }
            _ => STORAGE_ERR_GENERIC,
        },
        None => match e.kind() {
            io::ErrorKind::NotFound => STORAGE_ERR_NOT_FOUND,
            io::ErrorKind::AlreadyExists => STORAGE_ERR_EXIST,
            io::ErrorKind::PermissionDenied => STORAGE_ERR_ACCESS,
            io::ErrorKind::InvalidInput => STORAGE_ERR_NOT_VALID,
            _ => STORAGE_ERR_GENERIC,
        },
    }
}
