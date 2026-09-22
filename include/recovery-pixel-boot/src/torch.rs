//! torch — LM3644 flashlight control, fully native (no gpioset/i2cset forks).
//!
//! Discovery has two paths (P11 ab40c6f port):
//! 1. registered I2C client (/sys/bus/i2c/devices/*<match>, default "-0063")
//!    + flash pinctrl from the device tree (`samsung,pins` under a
//!    flash|torch path), GPIO chip by bank label (gs201/zuma/zumapro);
//! 2. LWIS fallback (malibu/laguna): the flash is a google,lwis-i2c-device
//!    whose camera driver never probes in recovery, so no I2C client and no
//!    samsung,pins exist. Bus/addr/enable-GPIO come straight from the
//!    flash@* device-tree node via phandle resolution.
//! GPIO uses the v1 uAPI (structs defined locally: bionic/libc exposes no
//! gpio ioctls); I2C reuses the i2c.rs transact helper.
//! Called by the Fox GUI as `recovery-pixel-boot torch on|off`.

use crate::i2c::{i2c_transact, open_i2c_dev};
use crate::ko_picker::log_msg;
use crate::props::get_prop;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

const TAG: &str = "torch";

// gpio v1 uAPI (stable ABI, defined locally — libc has no gpio ioctls).
// _IOWR(0xB4, 0x03, gpiohandle_request[364]) / _IOWR(0xB4, 0x09, u8[64]) /
// _IOR(0xB4, 0x01, gpiochip_info[68]).
// NOTE: REQUEST is NR 0x03 (0x02 is LINEINFO — wrong NR lands in the
// gpio_ioctl default branch = EINVAL before any field is read).
const GPIO_GET_CHIPINFO: libc::c_ulong = 0x8044B401;
const GPIOHANDLE_REQUEST: libc::c_ulong = 0xC16CB403;
const GPIOHANDLE_SET_VALUES: libc::c_ulong = 0xC040B409;
const GPIOHANDLE_REQUEST_OUTPUT: u32 = 1 << 1;

#[repr(C)]
struct GpioHandleRequest {
    lineoffsets: [u32; 64],
    flags: u32,
    default_values: [u8; 64],
    consumer_label: [u8; 32],
    lines: u32,
    fd: i32,
}

#[repr(C)]
struct GpioHandleData {
    values: [u8; 64],
}

#[repr(C)]
struct GpioChipInfo {
    name: [u8; 32],
    label: [u8; 32],
    lines: u32,
}

fn info(m: &str) {
    log_msg(TAG, "INFO", m);
}
fn err(m: &str) {
    log_msg(TAG, "ERROR", m);
}

fn cstr(bytes: &[u8]) -> String {
    let n = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..n]).into_owned()
}

/// "11-0063" -> (11, 0x63). Testable pure helper.
pub fn parse_i2c_devname(name: &str, suffix_match: &str) -> Option<(u32, u16)> {
    if !name.ends_with(suffix_match) {
        return None;
    }
    let (bus_str, addr_str) = name.split_once('-')?;
    if bus_str.is_empty() || !bus_str.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let bus: u32 = bus_str.parse().ok()?;
    let addr = u16::from_str_radix(addr_str, 16).ok()?;
    Some((bus, addr))
}

/// "gpp12-3" -> ("gpp12", 3). Testable pure helper.
pub fn parse_pin_name(pin: &str) -> Option<(String, u32)> {
    let (bank, off) = pin.rsplit_once('-')?;
    if bank.is_empty() {
        return None;
    }
    Some((bank.to_string(), off.parse().ok()?))
}

fn discover_i2c(suffix_match: &str) -> Result<(u32, u16), String> {
    let rd = std::fs::read_dir("/sys/bus/i2c/devices")
        .map_err(|e| format!("i2c devices unreadable: {e}"))?;
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(found) = parse_i2c_devname(&name, suffix_match) {
            return Ok(found);
        }
    }
    Err(format!("LM3644 (*{suffix_match}) not found on any I2C bus"))
}

fn visit_pins(dir: &Path, out: &mut Vec<PathBuf>) {
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return,
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            visit_pins(&p, out);
        } else if p.file_name().and_then(|n| n.to_str()) == Some("samsung,pins") {
            out.push(p);
        }
    }
}

fn discover_gpio(alternatives: &[String]) -> Result<(PathBuf, u32), String> {
    let mut pins = Vec::new();
    visit_pins(Path::new("/sys/firmware/devicetree/base"), &mut pins);
    for pins_file in &pins {
        let ps = pins_file.to_string_lossy();
        if !alternatives.iter().any(|a| ps.contains(a)) {
            continue;
        }
        let raw = std::fs::read(pins_file).map_err(|e| format!("pins unreadable: {e}"))?;
        for chunk in raw.split(|&b| b == 0) {
            let line = String::from_utf8_lossy(chunk).trim().to_string();
            if !line.contains('-') {
                continue;
            }
            if let Some((bank, off)) = parse_pin_name(&line) {
                let chip = find_gpiochip(&bank)
                    .ok_or_else(|| format!("no gpiochip for bank {bank}"))?;
                return Ok((chip, off));
            }
        }
    }
    Err("flash pinctrl not found in device tree".to_string())
}

fn find_gpiochip(bank: &str) -> Option<PathBuf> {
    for n in 0..32u32 {
        let node = PathBuf::from(format!("/dev/gpiochip{n}"));
        if !node.exists() {
            if n > 0 {
                // Chips are densely numbered from 0; first gap ends the scan.
                // (Keep scanning a little further — some kernels skip.)
            }
            continue;
        }
        let f = std::fs::OpenOptions::new().read(true).open(&node).ok()?;
        let mut info = GpioChipInfo {
            name: [0; 32],
            label: [0; 32],
            lines: 0,
        };
        // SAFETY: info is a valid gpiochip_info out-param; request constant
        // is the stable GPIO_GET_CHIPINFO ABI number.
        let rc = unsafe { libc::ioctl(f.as_raw_fd(), GPIO_GET_CHIPINFO as _, &mut info) };
        if rc == 0 && cstr(&info.label).contains(bank) {
            return Some(node);
        }
    }
    None
}

/// Drive one GPIO output line (v1 uAPI): request as output with value, hold
/// briefly for wake-up, then release. Mirrors `gpioset CHIP OFFSET=value`.
fn gpio_drive(chip: &Path, offset: u32, value: u8) -> Result<(), String> {
    // O_RDWR like libgpiod's chip open (mutation-capable fd).
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(chip)
        .map_err(|e| format!("gpiochip open failed: {e}"))?;
    let mut label = [0u8; 32];
    let tag = b"fox-torch";
    label[..tag.len()].copy_from_slice(tag);
    let mut req = GpioHandleRequest {
        lineoffsets: [0; 64],
        flags: GPIOHANDLE_REQUEST_OUTPUT,
        default_values: [0; 64],
        consumer_label: label,
        lines: 1,
        fd: -1,
    };
    req.lineoffsets[0] = offset;
    req.default_values[0] = value;
    // SAFETY: req is a valid gpiohandle_request; request constant is the
    // stable GPIOHANDLE_REQUEST ABI number.
    let rc = unsafe { libc::ioctl(f.as_raw_fd(), GPIOHANDLE_REQUEST as _, &mut req) };
    if rc < 0 {
        return Err(format!(
            "gpio request failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    if req.fd < 0 {
        return Err("gpio request returned no fd".to_string());
    }
    // v1 confirms the value via SET_VALUES (covers kernels that ignore
    // default_values on request).
    let mut data = GpioHandleData { values: [0; 64] };
    data.values[0] = value;
    // SAFETY: data is a valid 64-byte gpiohandle_data for our own handle fd.
    let rc = unsafe { libc::ioctl(req.fd, GPIOHANDLE_SET_VALUES as _, &mut data) };
    // SAFETY: close(2) on our own fd; return value intentionally ignored.
    unsafe {
        libc::close(req.fd);
    }
    if rc < 0 {
        return Err(format!(
            "gpio set failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

struct TorchHw {
    bus: u32,
    addr: u16,
    chip: PathBuf,
    gpio_off: u32,
}

/// Discovery cache: bus/addr/chip/offset never change across boots.
/// First toggle pays the full sysfs walk, later ones validate 4 nodes.
const HW_CACHE: &str = "/dev/.fox_torch_hw";

fn load_cache() -> Option<TorchHw> {
    let text = std::fs::read_to_string(HW_CACHE).ok()?;
    let mut it = text.split_whitespace();
    let bus: u32 = it.next()?.parse().ok()?;
    let addr = u16::from_str_radix(it.next()?, 16).ok()?;
    // "-" sentinel = LWIS discovery with no enable GPIO (no drive).
    let chip_str = it.next()?;
    let chip = if chip_str == "-" {
        PathBuf::new()
    } else {
        PathBuf::from(chip_str)
    };
    let gpio_off: u32 = it.next()?.parse().ok()?;
    if !Path::new(&format!("/dev/i2c-{bus}")).exists()
        || (!chip.as_os_str().is_empty() && !chip.exists())
    {
        return None;
    }
    Some(TorchHw {
        bus,
        addr,
        chip,
        gpio_off,
    })
}

fn store_cache(hw: &TorchHw) {
    let chip_str = if hw.chip.as_os_str().is_empty() {
        "-".to_string()
    } else {
        hw.chip.display().to_string()
    };
    let _ = std::fs::write(
        HW_CACHE,
        format!("{} {:x} {} {}\n", hw.bus, hw.addr, chip_str, hw.gpio_off),
    );
}

/// Big-endian u32 from a DT property blob (phandles, i2c-addr, gpio cells).
/// Testable pure helper.
pub fn parse_be_u32(raw: &[u8]) -> Option<u32> {
    if raw.len() < 4 {
        return None;
    }
    Some(u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

/// Match a phandle value to a linux i2c-N bus number by comparing each
/// /sys/bus/i2c/devices/i2c-*/of_node/phandle. Base-path parameterized
/// for tests. Port of the torch_ctl.sh (P11 ab40c6f) loop.
pub fn find_i2c_bus_by_phandle(phandle: u32, i2c_base: &Path) -> Option<u32> {
    let rd = std::fs::read_dir(i2c_base).ok()?;
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("i2c-") {
            continue;
        }
        // NOTE: plain `continue` (never `?`) on every per-entry failure:
        // one adapter without of_node/phandle must not abort the scan
        // (kodiak: first entries lack it, the flash bus sorts later).
        let bus: u32 = match name[4..].parse() {
            Ok(b) => b,
            Err(_) => continue,
        };
        // of_node may be a symlink; join() follows it either way.
        let ph = match std::fs::read(entry.path().join("of_node/phandle")) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if parse_be_u32(&ph) == Some(phandle) {
            return Some(bus);
        }
    }
    None
}

/// Match a phandle value to a gpiochip name (e.g. "gpiochip1") via
/// /sys/bus/gpio/devices/gpiochip*/of_node/phandle. Port of ab40c6f.
pub fn find_gpiochip_by_phandle(phandle: u32, gpio_base: &Path) -> Option<String> {
    let rd = std::fs::read_dir(gpio_base).ok()?;
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("gpiochip") {
            continue;
        }
        // Same no-`?`-in-loop rule as the i2c scan above.
        let ph = match std::fs::read(entry.path().join("of_node/phandle")) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if parse_be_u32(&ph) == Some(phandle) {
            return Some(name);
        }
    }
    None
}

fn visit_flash_nodes(dir: &Path, out: &mut Vec<PathBuf>) {
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return,
    };
    for e in rd.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        if p
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("flash@"))
            .unwrap_or(false)
        {
            out.push(p.clone());
        }
        visit_flash_nodes(&p, out);
    }
}

/// LWIS fallback (malibu/laguna, P11 ab40c6f port): the flash is a
/// google,lwis-i2c-device with no registered I2C client. Bus/addr come
/// from the flash@* node's i2c-bus (phandle) + i2c-addr (BE u32, low byte
/// is the 7-bit address); enable-gpios (<phandle line flags>) resolves to
/// a gpiochip the same way. GPIO is optional: LWIS would assert enable on
/// device-open, in recovery nothing does, so torch_on drives it high —
/// but an absent/unresolvable enable just means no GPIO drive (empty
/// chip path), not a discovery failure.
fn discover_lwis_flash() -> Result<TorchHw, String> {
    discover_lwis_flash_under(
        Path::new("/sys/firmware/devicetree/base"),
        Path::new("/sys/bus/i2c/devices"),
        Path::new("/sys/bus/gpio/devices"),
    )
}

fn discover_lwis_flash_under(dt_base: &Path, i2c_base: &Path, gpio_base: &Path) -> Result<TorchHw, String> {
    let mut nodes = Vec::new();
    visit_flash_nodes(dt_base, &mut nodes);
    for node in &nodes {
        let compat = std::fs::read(node.join("compatible")).unwrap_or_default();
        if !compat.windows(b"lwis-i2c-device".len()).any(|w| w == b"lwis-i2c-device") {
            continue;
        }
        let addr_raw = std::fs::read(node.join("i2c-addr"))
            .map_err(|_| "LWIS flash node has no i2c-addr".to_string())?;
        let bus_raw = std::fs::read(node.join("i2c-bus"))
            .map_err(|_| "LWIS flash node has no i2c-bus".to_string())?;
        // i2c-addr is a BE u32; the low byte is the 7-bit address.
        let addr = (parse_be_u32(&addr_raw).ok_or("bad i2c-addr")? & 0xff) as u16;
        let bus_ph = parse_be_u32(&bus_raw).ok_or("bad i2c-bus phandle")?;
        let bus = find_i2c_bus_by_phandle(bus_ph, i2c_base)
            .ok_or_else(|| format!("no i2c bus for phandle 0x{bus_ph:x}"))?;
        // enable-gpios is optional; cells are <phandle line flags>.
        let mut chip = PathBuf::new();
        let mut gpio_off = 0u32;
        if let Ok(eg) = std::fs::read(node.join("enable-gpios")) {
            if eg.len() >= 8 {
                if let (Some(en_ph), Some(off)) = (parse_be_u32(&eg[0..]), parse_be_u32(&eg[4..])) {
                    if let Some(name) = find_gpiochip_by_phandle(en_ph, gpio_base) {
                        chip = PathBuf::from(format!("/dev/{name}"));
                        gpio_off = off;
                    } else {
                        info(&format!("LWIS enable gpiochip for phandle 0x{en_ph:x} not found, no GPIO drive"));
                    }
                }
            }
        } else {
            info("LWIS flash node has no enable-gpios, no GPIO drive");
        }
        info(&format!(
            "LWIS flash -> I2C bus {bus} addr 0x{addr:02x}, enable {}{}",
            if chip.as_os_str().is_empty() {
                "(none)".to_string()
            } else {
                chip.display().to_string()
            },
            if chip.as_os_str().is_empty() {
                String::new()
            } else {
                format!(" line {gpio_off}")
            },
        ));
        return Ok(TorchHw {
            bus,
            addr,
            chip,
            gpio_off,
        });
    }
    Err("no lwis-i2c-device flash@ node in device tree".to_string())
}

fn discover(i2c_match: &str, pinctrl_alts: &[String]) -> Result<TorchHw, String> {
    if let Some(hw) = load_cache() {
        return Ok(hw);
    }
    match discover_i2c(i2c_match) {
        Ok((bus, addr)) => {
            let (chip, gpio_off) = discover_gpio(pinctrl_alts)?;
            info(&format!(
                "HW found: I2C bus {bus} addr 0x{addr:02x}, GPIO {} offset {gpio_off}",
                chip.display()
            ));
            let hw = TorchHw {
                bus,
                addr,
                chip,
                gpio_off,
            };
            store_cache(&hw);
            Ok(hw)
        }
        Err(client_err) => {
            // No registered LM3644 client (LWIS camera stack never probes
            // in recovery on malibu/laguna) — try the device-tree path.
            info(&format!("{client_err}, trying LWIS device-tree fallback"));
            let hw = discover_lwis_flash().map_err(|e| format!("{client_err}; {e}"))?;
            info(&format!(
                "HW found (LWIS): I2C bus {} addr 0x{:02x}, GPIO {}{}",
                hw.bus,
                hw.addr,
                if hw.chip.as_os_str().is_empty() {
                    "(none)".to_string()
                } else {
                    hw.chip.display().to_string()
                },
                if hw.chip.as_os_str().is_empty() {
                    String::new()
                } else {
                    format!(" offset {}", hw.gpio_off)
                },
            ));
            store_cache(&hw);
            Ok(hw)
        }
    }
}

fn i2c_do(bus: u32, addr: u16, payload: &[u8]) -> Result<(), String> {
    let f = open_i2c_dev(bus)?;
    i2c_transact(f.as_raw_fd(), addr, payload)
}

pub fn run_torch(action: &str) -> Result<(), String> {
    let code = get_prop("ro.hardware");
    let cfg = crate::config::load_device_config(&code).unwrap_or_default();
    let alts: Vec<String> = cfg
        .torch_pinctrl_match
        .split('|')
        .map(|s| s.to_string())
        .collect();
    match action {
        "on" => {
            let hw = discover(&cfg.torch_i2c_match, &alts).map_err(|e| {
                err(&format!("aborting torch ON: {e}"));
                e
            })?;
            // LWIS without enable GPIO: skip the drive, the chip is
            // already out of reset once probed (or has no HWEN line).
            if !hw.chip.as_os_str().is_empty() {
                info("waking flash chip via GPIO");
                gpio_drive(&hw.chip, hw.gpio_off, 1)?;
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            info("sending I2C commands");
            i2c_do(hw.bus, hw.addr, &[0x05, 0x3F])?;
            i2c_do(hw.bus, hw.addr, &[0x01, 0x0B])?;
            info("torch is ON");
            Ok(())
        }
        "off" => {
            let hw = discover(&cfg.torch_i2c_match, &alts).map_err(|e| {
                err(&format!("aborting torch OFF: {e}"));
                e
            })?;
            info("turning off LED via I2C");
            i2c_do(hw.bus, hw.addr, &[0x01, 0x00])?;
            if !hw.chip.as_os_str().is_empty() {
                info("chip to sleep via GPIO");
                gpio_drive(&hw.chip, hw.gpio_off, 0)?;
            }
            info("torch is OFF");
            Ok(())
        }
        _ => Err("usage: torch on|off".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i2c_name_parse() {
        assert_eq!(parse_i2c_devname("11-0063", "-0063"), Some((11, 0x63)));
        assert_eq!(parse_i2c_devname("11-0025", "-0063"), None);
        assert_eq!(parse_i2c_devname("uevent", "-0063"), None);
    }

    #[test]
    fn pin_name_parse() {
        assert_eq!(
            parse_pin_name("gpp12-3"),
            Some(("gpp12".to_string(), 3))
        );
        assert_eq!(parse_pin_name("nopin"), None);
    }

    #[test]
    fn gpio_struct_sizes_match_abi() {
        assert_eq!(std::mem::size_of::<GpioHandleRequest>(), 364);
        assert_eq!(std::mem::size_of::<GpioHandleData>(), 64);
        assert_eq!(std::mem::size_of::<GpioChipInfo>(), 68);
        assert_eq!(GPIOHANDLE_REQUEST, 0xC16CB403);
    }

    #[test]
    fn be_u32_parses_big_endian() {
        assert_eq!(parse_be_u32(&[0x00, 0x00, 0x00, 0x63]), Some(0x63));
        assert_eq!(parse_be_u32(&[0x00, 0x00, 0x00]), None);
        assert_eq!(parse_be_u32(&[]), None);
        // i2c-addr low byte is the 7-bit address.
        assert_eq!((parse_be_u32(&[0x00, 0x00, 0x00, 0x63]).unwrap() & 0xff) as u16, 0x63);
    }

    #[test]
    fn lwis_fallback_resolves_bus_addr_gpio() {
        use std::fs;
        let d = std::env::temp_dir().join(format!("fox_test_lwis_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        // DT: soc/flash@0 with lwis compatible, addr 0x63, bus phandle 0x11,
        // enable-gpios <0x22 line 24 flags 0>.
        let flash = d.join("dt/soc/flash@0");
        fs::create_dir_all(&flash).unwrap();
        fs::write(flash.join("compatible"), b"google,lwis-i2c-device\0").unwrap();
        fs::write(flash.join("i2c-addr"), [0x00, 0x00, 0x00, 0x63]).unwrap();
        fs::write(flash.join("i2c-bus"), [0x00, 0x00, 0x00, 0x11]).unwrap();
        fs::write(flash.join("enable-gpios"), [0x00, 0x00, 0x00, 0x22, 0x00, 0x00, 0x00, 0x18, 0x00, 0x00, 0x00, 0x00]).unwrap();
        // i2c-5/of_node/phandle = 0x11.
        let i2c5 = d.join("i2c/i2c-5/of_node");
        fs::create_dir_all(&i2c5).unwrap();
        fs::write(i2c5.join("phandle"), [0x00, 0x00, 0x00, 0x11]).unwrap();
        // gpiochip1/of_node/phandle = 0x22.
        let gc1 = d.join("gpio/gpiochip1/of_node");
        fs::create_dir_all(&gc1).unwrap();
        fs::write(gc1.join("phandle"), [0x00, 0x00, 0x00, 0x22]).unwrap();
        let hw = discover_lwis_flash_under(
            &d.join("dt"),
            &d.join("i2c"),
            &d.join("gpio"),
        )
        .unwrap();
        assert_eq!((hw.bus, hw.addr), (5, 0x63));
        assert_eq!(hw.chip, PathBuf::from("/dev/gpiochip1"));
        assert_eq!(hw.gpio_off, 24);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn lwis_fallback_no_enable_gpio_still_resolves() {
        use std::fs;
        let d = std::env::temp_dir().join(format!("fox_test_lwisnogpio_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let flash = d.join("dt/soc/flash@0");
        fs::create_dir_all(&flash).unwrap();
        fs::write(flash.join("compatible"), b"google,lwis-i2c-device\0").unwrap();
        fs::write(flash.join("i2c-addr"), [0x00, 0x00, 0x00, 0x63]).unwrap();
        fs::write(flash.join("i2c-bus"), [0x00, 0x00, 0x00, 0x11]).unwrap();
        let i2c5 = d.join("i2c/i2c-5/of_node");
        fs::create_dir_all(&i2c5).unwrap();
        fs::write(i2c5.join("phandle"), [0x00, 0x00, 0x00, 0x11]).unwrap();
        fs::create_dir_all(d.join("gpio")).unwrap();
        let hw = discover_lwis_flash_under(
            &d.join("dt"),
            &d.join("i2c"),
            &d.join("gpio"),
        )
        .unwrap();
        assert_eq!((hw.bus, hw.addr), (5, 0x63));
        assert!(hw.chip.as_os_str().is_empty());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn lwis_fallback_rejects_non_lwis_flash() {
        use std::fs;
        let d = std::env::temp_dir().join(format!("fox_test_lwisrej_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let flash = d.join("dt/soc/flash@0");
        fs::create_dir_all(&flash).unwrap();
        fs::write(flash.join("compatible"), b"some,other-flash\0").unwrap();
        fs::create_dir_all(d.join("i2c")).unwrap();
        fs::create_dir_all(d.join("gpio")).unwrap();
        assert!(discover_lwis_flash_under(
            &d.join("dt"),
            &d.join("i2c"),
            &d.join("gpio"),
        )
        .is_err());
        let _ = fs::remove_dir_all(&d);
    }
}
