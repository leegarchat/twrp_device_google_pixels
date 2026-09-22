//! i2c — MAX77759 TCPC USB-switch control.
//!
//! Port of the tail of otg_patch.sh: discover the TCPC on its I2C bus via
//! sysfs, grab the address with I2C_SLAVE_FORCE, write USBSW_CTRL(0x93) <- USBSW_CONNECT(0x09).

use std::ffi::CString;
use std::fs::OpenOptions;
use std::io::Error;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::Path;

// bionic ioctl(2) takes a 32-bit request; glibc takes c_ulong.
#[cfg(target_os = "android")]
const I2C_SLAVE_FORCE: libc::c_int = 0x0706;
#[cfg(not(target_os = "android"))]
const I2C_SLAVE_FORCE: libc::c_ulong = 0x0706;
const I2C_MAJOR: u32 = 89;
const USBSW_CTRL_REG: u8 = 0x93;
const USBSW_CONNECT: u8 = 0x09;

#[inline]
fn make_dev(major: u32, minor: u32) -> libc::dev_t {
    (((major as libc::dev_t) & 0xfff) << 8) | ((minor as libc::dev_t) & 0xff)
}

/// Scan a sysfs driver dir for `<bus>-<addr>` entries (e.g. "11-0025").
/// Returns the first hit. Separated for host testability.
pub fn find_tcpc_dev(sysfs_dir: &Path) -> Option<(u32, u16)> {
    let entries = std::fs::read_dir(sysfs_dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let (bus_str, addr_str) = match name.split_once('-') {
            Some(p) => p,
            None => continue,
        };
        if bus_str.is_empty() || !bus_str.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        // addr must be non-empty hex ("0025", "25", ...).
        if addr_str.is_empty() || !addr_str.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        if let (Ok(bus), Ok(addr)) = (
            bus_str.parse::<u32>(),
            u16::from_str_radix(addr_str, 16),
        ) {
            return Some((bus, addr));
        }
    }
    None
}

pub fn patch_max77759_i2c_with_driver(driver: &str) -> Result<(), String> {
    // Fox (laguna): the TCPC driver dirname differs per SoC generation
    // and may not match any known spelling (laguna: no max77759* dir at
    // all). Strategy: configured name first, known alternates second,
    // then auto-discover any i2c driver dir whose name hints at a
    // type-C port controller. First hit wins.
    let mut names: Vec<String> = vec![driver.to_string()];
    for alt in ["max77759tcpc", "max77759-tcpc", "max77759tcpc-spmi"] {
        if !names.iter().any(|n| n == alt) {
            names.push(alt.to_string());
        }
    }
    if let Ok(rd) = std::fs::read_dir("/sys/bus/i2c/drivers") {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let low = name.to_lowercase();
            if (low.contains("tcpc") || low.contains("77759") || low.contains("typec"))
                && !names.iter().any(|n| n == &name)
            {
                names.push(name);
            }
        }
    }
    let mut last_err = String::new();
    for name in &names {
        let tcpc_dir = format!("/sys/bus/i2c/drivers/{name}");
        let tcpc_sysfs = Path::new(&tcpc_dir);
        match find_tcpc_dev(tcpc_sysfs) {
            Some((bus, addr)) => return patch_tcpc_at(name, bus, addr),
            None => last_err = format!("{name} TCPC not found in sysfs"),
        }
    }
    Err(format!("no TCPC driver matched (tried: {}); {last_err}", names.join(",")))
}

/// Former body of patch_max77759_i2c_with_driver: drive the switch at a
/// resolved (driver, bus, addr). Split out so the fallback loop above
/// stays readable.
fn patch_tcpc_at(_driver: &str, bus: u32, addr: u16) -> Result<(), String> {

    let dev_node = format!("/dev/i2c-{bus}");
    let dev_path = Path::new(&dev_node);

    // ueventd normally creates the node; mknod is a fallback only.
    if !dev_path.exists() {
        let dev_id = make_dev(I2C_MAJOR, bus);
        let c_node = CString::new(dev_node.clone()).map_err(|_| "CString error".to_string())?;
        let mode = (libc::S_IFCHR | 0o660) as libc::mode_t;
        // SAFETY: node path is a valid NUL-terminated C string, mode/dev
        // describe a character device; mknod has no thread-safety concerns.
        let res = unsafe { libc::mknod(c_node.as_ptr(), mode, dev_id) };
        if res != 0 {
            return Err(format!(
                "mknod failed for {dev_node}: {}",
                Error::last_os_error()
            ));
        }
    }

    let file = open_i2c_dev(bus)?;
    let fd = file.as_raw_fd();

    // SAFETY: fd is our own open /dev/i2c-N, request is I2C_SLAVE_FORCE with
    // a u16 slave address: the standard i2c-dev ioctl contract.
    if unsafe { libc::ioctl(fd, I2C_SLAVE_FORCE, addr as libc::c_ulong) } < 0 {
        return Err(format!(
            "ioctl I2C_SLAVE_FORCE (0x{addr:02x}) failed: {}",
            Error::last_os_error()
        ));
    }

    i2c_write(fd, &[USBSW_CTRL_REG, USBSW_CONNECT])
}

/// Open /dev/i2c-<bus> (mknod fallback if ueventd hasn't created it).
/// Returns the open File; the fd stays valid while it is alive.
pub fn open_i2c_dev(bus: u32) -> Result<std::fs::File, String> {
    let dev_node = format!("/dev/i2c-{bus}");
    let dev_path = Path::new(&dev_node);

    // ueventd normally creates the node; mknod is a fallback only.
    if !dev_path.exists() {
        let dev_id = make_dev(I2C_MAJOR, bus);
        let c_node = CString::new(dev_node.clone()).map_err(|_| "CString error".to_string())?;
        let mode = (libc::S_IFCHR | 0o660) as libc::mode_t;
        // SAFETY: node path is a valid NUL-terminated C string, mode/dev
        // describe a character device; mknod has no thread-safety concerns.
        let res = unsafe { libc::mknod(c_node.as_ptr(), mode, dev_id) };
        if res != 0 {
            return Err(format!(
                "mknod failed for {dev_node}: {}",
                Error::last_os_error()
            ));
        }
    }

    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(dev_path)
        .map_err(|e| format!("cannot open {dev_node}: {e}"))
}

/// Grab an I2C slave address on an open i2c-dev fd and write payload bytes.
pub fn i2c_transact(fd: std::os::unix::io::RawFd, addr: u16, payload: &[u8]) -> Result<(), String> {
    // SAFETY: fd is our own open /dev/i2c-N, request is I2C_SLAVE_FORCE with
    // a u16 slave address: the standard i2c-dev ioctl contract.
    if unsafe { libc::ioctl(fd, I2C_SLAVE_FORCE, addr as libc::c_ulong) } < 0 {
        return Err(format!(
            "ioctl I2C_SLAVE_FORCE (0x{addr:02x}) failed: {}",
            Error::last_os_error()
        ));
    }
    i2c_write(fd, payload)
}

/// Write raw bytes to an already-addressed i2c-dev fd.
pub fn i2c_write(fd: std::os::unix::io::RawFd, payload: &[u8]) -> Result<(), String> {
    // SAFETY: payload points to a live slice, fd is ours; partial writes
    // are treated as failure by the length check below.
    let written =
        unsafe { libc::write(fd, payload.as_ptr() as *const libc::c_void, payload.len()) };
    if written != payload.len() as isize {
        return Err(format!("I2C write failed: {}", Error::last_os_error()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_bus_addr_in_real_tempdir() {
        let d = std::env::temp_dir().join(format!("fox_test_tcpc_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("11-0025")).unwrap();
        std::fs::write(d.join("uevent"), b"junk").unwrap();
        assert_eq!(find_tcpc_dev(&d), Some((11, 0x25)));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rejects_garbage_entries() {
        let d = std::env::temp_dir().join(format!("fox_test_tcpc2_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::create_dir_all(d.join("not-a-device-zz")).unwrap();
        std::fs::create_dir_all(d.join("module")).unwrap();
        assert_eq!(find_tcpc_dev(&d), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn missing_dir_is_none() {
        assert_eq!(
            find_tcpc_dev(Path::new("/definitely/not/here")),
            None
        );
    }
}
