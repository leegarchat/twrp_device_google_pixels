//! Critical-partition backup: dump target-slot residents to tmp before
//! the first write, restore them on batch failure. Weights 2+4 only —
//! super (1) and modem (3) are excluded by design.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

/// Dump a whole block node to a file. Sizes are small by construction
/// (boot set + bootloaders/vbmeta); the caller filters the set.
pub fn dump(node: &str, file: &str, size: u64) -> Result<(), String> {
    let mut src = File::open(node).map_err(|e| format!("open {node}: {e}"))?;
    let dst = File::create(file).map_err(|e| format!("create {file}: {e}"))?;
    let mut dst = dst;
    let mut left = size;
    let mut buf = vec![0u8; 1 << 20];
    while left > 0 {
        let n = (left as usize).min(buf.len());
        src.read_exact(&mut buf[..n])
            .map_err(|e| format!("read {node}: {e}"))?;
        dst.write_all(&buf[..n])
            .map_err(|e| format!("write {file}: {e}"))?;
        left -= n as u64;
    }
    dst.flush().map_err(|e| format!("flush {file}: {e}"))?;
    Ok(())
}

/// Restore a dumped file back over its node + fsync the node.
pub fn restore(file: &str, node: &str) -> Result<(), String> {
    let mut src = File::open(file).map_err(|e| format!("open {file}: {e}"))?;
    let mut dst = File::options()
        .write(true)
        .open(node)
        .map_err(|e| format!("open {node}: {e}"))?;
    dst.seek(SeekFrom::Start(0))
        .map_err(|e| format!("seek {node}: {e}"))?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = src.read(&mut buf).map_err(|e| format!("read {file}: {e}"))?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n])
            .map_err(|e| format!("write {node}: {e}"))?;
    }
    dst.flush().map_err(|e| format!("flush {node}: {e}"))?;
    // fsync the node so the restore survives a power cut
    use std::os::unix::io::AsRawFd;
    if libc_fsync(dst.as_raw_fd()) != 0 {
        return Err(format!("fsync {node} failed"));
    }
    Ok(())
}

extern "C" {
    fn fsync(fd: i32) -> i32;
}

fn libc_fsync(fd: i32) -> i32 {
    unsafe { fsync(fd) }
}

/// fsync a node by path (no libc crate: raw binding above).
pub fn fsync_node(node: &str) -> Result<(), String> {
    use std::os::unix::io::AsRawFd;
    let f = File::open(node).map_err(|e| format!("open {node}: {e}"))?;
    if libc_fsync(f.as_raw_fd()) != 0 {
        return Err(format!("fsync {node} failed"));
    }
    Ok(())
}
