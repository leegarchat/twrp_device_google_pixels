//! UFS RPMB transport over SCSI Generic (SG_IO).
//!
//! Mirrors the SECURITY PROTOCOL IN/OUT flow of the original C proxy
//! (opcodes 0xA2/0xB5, protocol 0xEC) and adds dynamic RPMB node discovery:
//! an explicit `-r` path wins; otherwise `/tmp/.rpmb_sg_dev` is consulted,
//! falling back to a `/sys/class/scsi_generic` scan constrained to the UFS
//! host controller and the RPMB well-known LUN.

use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

/// SG_IO ioctl number (fixed, no size field).
const SG_IO: libc::c_ulong = 0x2285;
/// SG_GET_VERSION_NUM ioctl number.
const SG_GET_VERSION_NUM: libc::c_ulong = 0x2282;
/// Minimum sg driver version accepted for RPMB duty.
const RPMB_MIN_SG_VERSION: libc::c_int = 30000;

/// Cache file holding the last successfully discovered sg node.
const RPMB_CACHE_PATH: &str = "/tmp/.rpmb_sg_dev";
/// sysfs class listing all SCSI generic nodes.
const SG_CLASS_PATH: &str = "/sys/class/scsi_generic";

const SG_DXFER_TO_DEV: libc::c_int = -2;
const SG_DXFER_FROM_DEV: libc::c_int = -3;

/// Timeout per SG command, ms (matches the C proxy).
const SG_TIMEOUT_MS: u32 = 20000;

/// SECURITY PROTOCOL IN (read) / OUT (write) opcodes for RPMB (protocol 0xEC).
const CDB_RPMB_READ: u8 = 0xA2;
const CDB_RPMB_WRITE: u8 = 0xB5;
const CDB_SECPROTO_RPMB: u8 = 0xEC;

/// Retries on UNIT ATTENTION (power-on/reset, ASC 0x29).
const UFS_WRITE_RETRY: u32 = 1;
const UFS_READ_RETRY: u32 = 3;

/// Decimal rendering of the UFS RPMB well-known LUN id 0xC144.
const UFS_RPMB_WLUN_SUFFIX: &str = "49476";

/// Linux `sg_io_hdr` (32-bit `int` fields kept exact; `#[repr(C)]` handles
/// pointer-width differences between arm/arm64).
#[repr(C)]
struct SgIoHdr {
    interface_id: libc::c_int,
    dxfer_direction: libc::c_int,
    cmd_len: libc::c_uchar,
    mx_sb_len: libc::c_uchar,
    iovec_count: libc::c_ushort,
    dxfer_len: libc::c_uint,
    dxferp: *mut libc::c_void,
    cmdp: *mut libc::c_uchar,
    sbp: *mut libc::c_uchar,
    timeout: libc::c_uint,
    flags: libc::c_uint,
    pack_id: libc::c_int,
    usr_ptr: *mut libc::c_void,
    status: libc::c_uchar,
    masked_status: libc::c_uchar,
    msg_status: libc::c_uchar,
    sb_len_wr: libc::c_uchar,
    host_status: libc::c_ushort,
    driver_status: libc::c_ushort,
    resid: libc::c_int,
    duration: libc::c_uint,
    info: libc::c_uint,
}

/// 12-byte SECURITY PROTOCOL CDB, transmitted big-endian on the wire.
#[repr(C, packed)]
struct SecProtoCdb {
    opcode: u8,
    sec_proto: u8,
    cdb_byte_2: u8,
    cdb_byte_3: u8,
    cdb_byte_4: u8,
    cdb_byte_5: u8,
    length_be: u32,
    cdb_byte_10: u8,
    ctrl: u8,
}

const _: () = assert!(std::mem::size_of::<SecProtoCdb>() == 12);

fn make_cdb(opcode: u8, xfer_len: u32) -> SecProtoCdb {
    SecProtoCdb {
        opcode,
        sec_proto: CDB_SECPROTO_RPMB,
        cdb_byte_2: 0,
        cdb_byte_3: 1,
        cdb_byte_4: 0,
        cdb_byte_5: 0,
        length_be: xfer_len.to_be(),
        cdb_byte_10: 0,
        ctrl: 0,
    }
}

/// Opened RPMB channel. Closes the sg fd on drop.
pub struct Rpmb {
    fd: OwnedFd,
}

impl Rpmb {
    /// Opens `dev` and verifies it is a usable sg node.
    pub fn open(dev: &str) -> io::Result<Rpmb> {
        let dev_c = CString::new(dev).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "rpmb device path contains NUL")
        })?;
        // Safety: path is a valid CString; flags are valid; fd checked below.
        let raw = unsafe { libc::open(dev_c.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: owned fd from a successful open.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let mut ver: libc::c_int = 0;
        // Safety: fd is valid; SG_GET_VERSION_NUM fits i32; &mut ver is a valid out-param.
        let rc = unsafe { libc::ioctl(fd.as_raw_fd(), SG_GET_VERSION_NUM as _, &mut ver) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        if ver < RPMB_MIN_SG_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{dev}: not a valid sg device (version {ver})"),
            ));
        }
        crate::logi!("sp", "{dev}: sg version {ver}");
        Ok(Rpmb { fd })
    }

    fn sg(&self, dir: libc::c_int, cdb: &SecProtoCdb, data: *mut u8, len: u32) -> io::Result<SgStatus> {
        let mut sense = [0u8; 32];
        let mut hdr = SgIoHdr {
            interface_id: b'S' as libc::c_int,
            dxfer_direction: dir,
            cmd_len: std::mem::size_of::<SecProtoCdb>() as libc::c_uchar,
            mx_sb_len: sense.len() as libc::c_uchar,
            iovec_count: 0,
            dxfer_len: len as libc::c_uint,
            dxferp: data as *mut libc::c_void,
            // CDB is only read by the kernel; cast away constness.
            cmdp: cdb as *const SecProtoCdb as *mut libc::c_uchar,
            sbp: sense.as_mut_ptr(),
            timeout: SG_TIMEOUT_MS,
            flags: 0,
            pack_id: 0,
            usr_ptr: std::ptr::null_mut(),
            status: 0,
            masked_status: 0,
            msg_status: 0,
            sb_len_wr: 0,
            host_status: 0,
            driver_status: 0,
            resid: 0,
            duration: 0,
            info: 0,
        };
        // Safety: fd is valid; SG_IO fits i32; &mut hdr points at a live sg_io_hdr.
        let rc = unsafe { libc::ioctl(self.fd.as_raw_fd(), SG_IO as _, &mut hdr) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(SgStatus::classify(&hdr, &sense))
    }

    /// Executes one RPMB_SEND transaction.
    ///
    /// `reliable`/`write` are the two write phases (may be empty); returns
    /// exactly `read_size` bytes read back from the RPMB well-known LUN.
    pub fn transact(
        &self,
        reliable: &[u8],
        write: &[u8],
        read_size: usize,
    ) -> io::Result<Vec<u8>> {
        if !reliable.is_empty() {
            let cdb = make_cdb(CDB_RPMB_WRITE, reliable.len() as u32);
            let mut tries = UFS_WRITE_RETRY + 1;
            loop {
                match self.sg(
                    SG_DXFER_TO_DEV,
                    &cdb,
                    reliable.as_ptr() as *mut u8,
                    reliable.len() as u32,
                )? {
                    SgStatus::Ok => break,
                    SgStatus::Retry if tries > 1 => {
                        tries -= 1;
                        continue;
                    }
                    SgStatus::Retry | SgStatus::Fail => {
                        crate::loge!("sp", "rpmb reliable-write SG_IO failed");
                        break;
                    }
                }
            }
        }
        if !write.is_empty() {
            // A pure read-phase write (no reliable part) gets more retries,
            // matching the C proxy's `is_req_write ? 0 : UFS_READ_RETRY`.
            let mut tries = if reliable.is_empty() { UFS_READ_RETRY + 1 } else { 1 };
            let cdb = make_cdb(CDB_RPMB_WRITE, write.len() as u32);
            loop {
                match self.sg(
                    SG_DXFER_TO_DEV,
                    &cdb,
                    write.as_ptr() as *mut u8,
                    write.len() as u32,
                )? {
                    SgStatus::Ok => break,
                    SgStatus::Retry if tries > 1 => {
                        tries -= 1;
                        continue;
                    }
                    SgStatus::Retry | SgStatus::Fail => {
                        crate::loge!("sp", "rpmb write SG_IO failed");
                        break;
                    }
                }
            }
        }
        let mut out = vec![0u8; read_size];
        if read_size > 0 {
            let cdb = make_cdb(CDB_RPMB_READ, read_size as u32);
            match self.sg(SG_DXFER_FROM_DEV, &cdb, out.as_mut_ptr(), read_size as u32)? {
                SgStatus::Ok | SgStatus::Fail => {}
                SgStatus::Retry => crate::logi!("sp", "rpmb read: unit attention, data may be stale"),
            }
        }
        Ok(out)
    }
}

/// Classified SG result: clean, transient UNIT ATTENTION, or hard failure.
enum SgStatus {
    Ok,
    /// Sense UNIT ATTENTION / power-on: worth exactly one re-issue.
    Retry,
    Fail,
}

impl SgStatus {
    fn classify(h: &SgIoHdr, sense: &[u8]) -> SgStatus {
        if h.status == 0 && h.host_status == 0 && h.driver_status == 0 {
            return SgStatus::Ok;
        }
        let n = h.sb_len_wr as usize;
        if n > 0 {
            let sb = &sense[..n.min(sense.len())];
            let resp = sb[0] & 0x7f;
            let (key, asc) = if resp >= 0x72 {
                // Descriptor format.
                (sb.get(1).copied().unwrap_or(0) & 0x0f, sb.get(2).copied().unwrap_or(0))
            } else if resp >= 0x70 {
                // Fixed format.
                (sb.get(2).copied().unwrap_or(0) & 0x0f, sb.get(12).copied().unwrap_or(0))
            } else {
                (0xff, 0xff)
            };
            if key == 0x00 || key == 0x0f {
                return SgStatus::Ok;
            }
            if key == 0x06 && asc == 0x29 {
                return SgStatus::Retry;
            }
        }
        crate::loge!(
            "sp",
            "SG_IO error: status={} masked={} host={} drv={}",
            h.status,
            h.masked_status,
            h.host_status,
            h.driver_status
        );
        SgStatus::Fail
    }
}

/// Resolves the RPMB sg node.
///
/// Precedence: explicit `-r` path > `/tmp/.rpmb_sg_dev` cache > sysfs scan.
/// A successful scan result is written back to the cache file (best effort).
pub fn resolve_device(explicit: Option<&str>) -> Result<String, String> {
    if let Some(dev) = explicit {
        if dev.is_empty() {
            return Err("empty -r rpmb device path".to_string());
        }
        return Ok(dev.to_string());
    }
    if let Ok(cached) = std::fs::read_to_string(RPMB_CACHE_PATH) {
        let cached = cached.trim().to_string();
        if !cached.is_empty() {
            if std::path::Path::new(&cached).exists() {
                crate::logi!("sp", "rpmb: using cached node {cached}");
                return Ok(cached);
            }
            crate::loge!("sp", "rpmb: stale cache entry {cached}, rescanning");
        }
    }
    let found = scan_sysfs().ok_or_else(|| "rpmb: no UFS RPMB sg node found".to_string())?;
    // Best effort: a missing /tmp must not fail the boot path.
    if let Err(e) = std::fs::write(RPMB_CACHE_PATH, format!("{found}\n")) {
        crate::loge!("sp", "rpmb: cannot write cache {RPMB_CACHE_PATH}: {e}");
    } else {
        crate::logi!("sp", "rpmb: cached node {found}");
    }
    Ok(found)
}

/// Scans `/sys/class/scsi_generic/sg*` for the UFS RPMB well-known LUN.
///
/// A candidate must: sit behind a UFS host controller (`ufshcd`/`ufs` in the
/// canonical device path), expose no `block/` child (RPMB has no block
/// interface), and carry the RPMB W-LUN id (path ending in 49476 = 0xC144,
/// or a `wlun` name component).
fn scan_sysfs() -> Option<String> {
    let dir = std::fs::read_dir(SG_CLASS_PATH).ok()?;
    let mut names: Vec<String> = dir
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("sg"))
        .collect();
    names.sort();
    for sg in names {
        let suffix: String = sg.chars().skip(2).collect();
        if suffix.is_empty() || !suffix.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let dev_link = format!("{SG_CLASS_PATH}/{sg}/device");
        let target = std::fs::read_link(&dev_link).ok()?.to_string_lossy().into_owned();
        // Must belong to the UFS controller: filters out USB sticks, etc.
        if !(target.contains("ufshcd") || target.contains("ufs")) {
            continue;
        }
        // RPMB W-LUN exposes no block interface.
        if std::path::Path::new(&format!("{dev_link}/block")).exists() {
            continue;
        }
        // LUN identity: .../lun-...49476 or a wlun component.
        let is_rpmb_wlun = target.ends_with(UFS_RPMB_WLUN_SUFFIX) || target.contains("wlun");
        if !is_rpmb_wlun {
            continue;
        }
        let node = format!("/dev/{sg}");
        if std::path::Path::new(&node).exists() {
            crate::logi!("sp", "rpmb: discovered {node} ({target})");
            return Some(node);
        }
    }
    None
}
