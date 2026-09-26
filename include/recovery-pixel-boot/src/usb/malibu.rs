//! usb::malibu — OTG implementation (Tensor G6: grizzly/cubs/kodiak/yogi).
//!
//! Faithful Rust port of the P11 reference `otg_yogi.sh` (83 lines,
//! `/home/leegarbook/android_builds/P11_ofox/recovery/root/system/bin/otg_yogi.sh`):
//! the kernel owns the Type-C mode (tcpm votes TCPCI:host on attach) and
//! `google-usb-role-sw` additionally needs the AoC's vote, which goes host
//! only once the AoC `usb_control` service runs — started by `aocd`. We
//! stage the device's OWN vendor runtime (`aocd` + 6 libs +
//! `aoc_usb_driver.ko`) into a tmpfs dir (survives a later vendor unmount),
//! insmod once, and relaunch the flaky `aocd` until `usb_control` appears.
//!
//! Local binary facts (grizzly factory `cd1a.260905.001.b1`, kodiak same
//! build; kodiak GKI kernel byte-identical, both `.ko` md5-identical —
//! cubs/yogi have no local factory zips, constants below are
//! grizzly-derived for them):
//! - GKI 6.12.69: `usb_role_switch`×41, `typec`×322, `tcpm-source-psy`×1,
//!   `dwc3_otg_host_ready`×0, `CHARGER_MODE`×0, `dwc3_exynos_otg_id`×0 →
//!   6.12-only family (no older kernel exists), shim dead by design, native
//!   role-switch path only. There is NOTHING to force: no role writes, no
//!   gvotable VBUS force (no `CHARGER_MODE` voter in the kernel).
//! - `aoc_usb_driver.ko`: `usb_control` service inside, `depends=aoc_core,
//!   gvotable`, vermagic `6.12.69-android16-6-g6a49175400c0-ab16238327-4k`
//!   (strict insmod: matches the running vendor kernel by construction —
//!   the factory GKI `boot.img` embeds an older `ab15835541` string, but the
//!   on-device kernel the modules load against carries the vendor string;
//!   the P11 script's plain `insmod` is field-proven, so strict flags here).
//! - `dwc3-google-aux.ko`: alias `google,dwc3-mbu-aux`,
//!   `desired_role`/`current_role` tracepoints, `depends=google_icc`.
//! - First-stage (`vendor_kernel_boot` ramdisk `modules.load`) already
//!   autoloads the whole stack before we run: `aoc_core`, `gvotable`,
//!   `google_icc`, `google-role-sw`, `dwc3-google`, `tcpci_max77759`,
//!   `usb_psy`, max77779 chargers. Our only insmod is `aoc_usb_driver.ko`
//!   (deps already live); guarded by `/sys/module/aoc_usb_driver` so a
//!   second run is a no-op. `modules.dep` in `vendor_dlkm` lists empty deps
//!   for both `.ko` (their providers live in the other partition).
//! - DTBO: `goog_usb_role_sw [google,usb-role-sw]` status=okay,
//!   `max77779pmic [maxim,max77779pmic-spmi]` (SPMI!), charger
//!   `google,cpm/chargers/max77779`, eUSB2 repeater `tiusb2e11`
//!   `[goog-eusb2-repeater]`, `max77759tcpc-spmi@4/connector`.
//! - Base DTB (`vendor_kernel_boot`): `dwc3@a210000` under
//!   `usb3@a200000` + `usb-role-switch` property → UDC `a210000.dwc3`
//!   (also P11 `runatinit.sh:185` + `twrp_malibu.flags` comment).
//! - Live-device sysfs truths (kodiak recovery logs, Sept 2026):
//!   role switches `/sys/class/usb_role/a200000.usb3-role-switch/role`
//!   (`device`/`host`) + `i2c-8-003e-eusb2-repeater-role-switch`;
//!   AoC platform device `a800000.aoc`; source psy
//!   `tcpm-source-psy-spmi-max77759tcpc`; VBUS
//!   `/sys/class/power_supply/usb/online`.
//! - `vendor.img` (EROFS): `bin/aocd` + all 6 `lib64/*.so` present at the
//!   exact P11 copy paths; `readelf -d aocd` NEEDED confirms the set
//!   (`liblog`/`libc`/`libm`/`libdl` come from the recovery ramdisk).
//! - `aoc_core.ko` strings carry `verify_aoc_responsive`/`responsive`/
//!   `services`; `aoc_usb_driver.ko` votes `USB_ROLE_HOST` on the
//!   `usb_data_role_votable` election (`VOTABLE_USB_DATA_ROLE="USB_DR_EL"`
//!   per the kodiak mirror `aoc/usb/aoc_usb_dev.c`, which also shows the
//!   `"usb_control"` service name and the `Fail to cast vote` retry).
//!
//! Deliberately NO i2c TCPC patch (`crate::i2c::patch_max77759_i2c_with_driver`
//! is NOT called): the port controller is an SPMI device
//! (`max77759tcpc-spmi@4`, first-stage `tcpci_max777x9_spmi.ko`,
//! `vendor_boot` cmdline `max77779_pmic*`/`spmi_smartdv`) — there is no I2C
//! client to grab (the old daemon logged `TCPC switch not configured` on
//! device for exactly this reason) and the stock driver self-manages the
//! data path. See `crate::i2c` docs.
//!
//! Success predicate: host is ACTIVE only when the controller data role
//! reads back `host` AND the TCPC source psy reads `online=1` — never the
//! role alone (a bare `role=host` readback with no sourced VBUS is not
//! host mode). `run_otg_patch` gates `sys.usb.patch_dwc3=1` (which starts
//! `otg_auto` in our rc) on the durable enabler — the live AoC vote
//! (`usb_control` present) — because an accessory may attach *after* boot;
//! failing patch at boot-with-nothing-attached would brick late attach.
//! The daemon then serves late attach by keeping the vote alive. Any staging
//! / insmod / vote failure fails closed: device mode, adb safe (we never
//! touch UDC, gadget, role or voter state — TCPC + role-sw negotiate modes
//! on their own).

use crate::ko_picker::{is_module_loaded, load_kernel_module, log_msg};
use crate::props::{get_prop, set_prop};
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::thread::sleep;
use std::time::Duration;

const TAG: &str = "otg";
/// Staging dir in tmpfs (P11 `otg_yogi.sh:17`: `RAM=/tmp/aoc`).
const RAM_DIR: &str = "/tmp/aoc";
/// Lib subdir (P11 `:17`: `mkdir -p "$RAM/lib"`).
const RAM_LIB: &str = "/tmp/aoc/lib";
/// Daemon binary name in RAM (P11 `:29`: `cp -f "$MV/bin/aocd" "$RAM/"`).
const RAM_AOCD: &str = "/tmp/aoc/aocd";
/// Module copy in RAM (P11 `:38`: `cp -f "$ko" "$RAM/"`).
const RAM_KO: &str = "/tmp/aoc/aoc_usb_driver.ko";
/// Module name for the `/sys/module` + `/proc/modules` guards (P11 `:69`).
const MOD_NAME: &str = "aoc_usb_driver";
/// sysfs guard: present once the module is loaded (P11 `:69`).
const SYS_MODULE: &str = "/sys/module/aoc_usb_driver";
/// Live vendor mount points (P11 `:20`: `MV=/dev/otg_mnt/vendor`,
/// `MD=/dev/otg_mnt/vdlkm`).
const MNT_VENDOR: &str = "/dev/otg_mnt/vendor";
const MNT_VDLKM: &str = "/dev/otg_mnt/vdlkm";
/// dm-mapper prefixes with the slot suffix appended (P11 `:28`,`:36`:
/// `/dev/block/mapper/vendor$slot`, `vendor_dlkm$slot`).
const MAP_VENDOR: &str = "/dev/block/mapper/vendor";
const MAP_VDLKM: &str = "/dev/block/mapper/vendor_dlkm";
/// Daemon source path on the live vendor (P11 `:29`: `$MV/bin/aocd`).
const VENDOR_AOCD: &str = "bin/aocd";
/// The 6 userspace libs (P11 `:30-32`):
/// `libaoc libbase libevent aoc_aconfig_flags_c_lib
/// libaconfig_storage_read_api_cc libc++` from `$MV/lib64/$l.so` —
/// verified present on grizzly `vendor.img` + `readelf -d aocd` NEEDED.
const AOC_LIBS: &[&str] = &[
    "libaoc",
    "libbase",
    "libevent",
    "aoc_aconfig_flags_c_lib",
    "libaconfig_storage_read_api_cc",
    "libc++",
];
/// AoC platform devices: `/sys/devices/platform/*.aoc` (P11 `:44-45`;
/// live device name `a800000.aoc`).
const AOC_PLATFORM: &str = "/sys/devices/platform";
/// Responsiveness gate node + expected content (P11 `:44`:
/// `verify_aoc_responsive` == `responsive`; strings live in `aoc_core.ko`).
const VERIFY_NODE: &str = "verify_aoc_responsive";
const RESPONSIVE: &str = "responsive";
/// Service list node (P11 `:45`: `find ... -name services`).
const SERVICES_NODE: &str = "services";
/// The service proving the AoC vote is live (P11 `:45`,`:70-76`:
/// `grep -q usb_control`; string lives in `aoc_usb_driver.ko`, service
/// registered in mirror `aoc/usb/aoc_usb_dev.c:361`).
const USB_CONTROL: &str = "usb_control";
/// Role-switch class (zuma.rs pattern; live entries
/// `a200000.usb3-role-switch`, `i2c-8-003e-eusb2-repeater-role-switch`).
const USB_ROLE_CLASS: &str = "/sys/class/usb_role";
/// Preferred role-switch entry: the DWC3 controller switch
/// (`a200000.usb3-role-switch/role`, live log).
const ROLE_PREFER: &str = "usb3";
/// Source psy class + exact psy name (live chg log:
/// `chg_psy_changed name=tcpm-source-psy-spmi-max77759tcpc`).
const PSY_CLASS: &str = "/sys/class/power_supply";
const SOURCE_PSY: &str = "tcpm-source-psy-spmi-max77759tcpc";
/// Stable symlink for `/usb_otg` (P11 `:78-80`, `twrp_malibu.flags`,
/// zuma.rs `OTG_USB_LINK`).
const OTG_USB_LINK: &str = "/dev/block/otg-usb";
/// `aocd` relaunch settle wait (P11 `:75`: `sleep 4`).
const AOCD_SETTLE_SECS: u64 = 4;
/// Supervisor tick (P11 `:82`: `sleep 3`).
const TICK_SECS: u64 = 3;
/// Patch-time staging retries: dm-mapper nodes appear late (P11 `:25-26`,
/// retried from the loop until they appear).
const STAGE_ATTEMPTS: u32 = 10;
/// Patch-time bounded `aocd` relaunch attempts (P11 `:60-63` relaunches a
/// fresh instance until `usb_control` appears; bounded here so the oneshot
/// patch terminates — the daemon keeps retrying afterwards).
const AOCD_ATTEMPTS: u32 = 6;

fn info(msg: &str) {
    log_msg(TAG, "INFO", msg);
    println!("otg: INFO: {msg}");
}

fn read_trim(p: &str) -> String {
    std::fs::read_to_string(p).unwrap_or_default().trim().to_string()
}

/// Slot suffix: `ro.boot.slot_suffix` wins, `/proc/bootconfig`
/// `androidboot.slot_suffix = "_a"` line is the fallback (P11 `:18-19`).
/// Pure over inputs (testable); live prop/file reads in [`slot_suffix`].
fn parse_slot_suffix(prop: &str, bootconfig: &str) -> String {
    let p = prop.trim();
    if !p.is_empty() {
        return p.to_string();
    }
    for line in bootconfig.lines() {
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == "androidboot.slot_suffix" {
                let s = v.trim().trim_matches('"').trim().to_string();
                if !s.is_empty() {
                    return s;
                }
            }
        }
    }
    String::new()
}

/// Live slot suffix.
fn slot_suffix() -> String {
    let prop = get_prop("ro.boot.slot_suffix");
    let bc = std::fs::read_to_string("/proc/bootconfig").unwrap_or_default();
    parse_slot_suffix(&prop, &bc)
}

/// One recursive walk collecting files named `want` (P11 `:37`:
/// `find "$MD" -name aoc_usb_driver.ko | head -1`).
fn find_named(root: &Path, want: &str, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            find_named(&p, want, out);
        } else if p.file_name().and_then(|n| n.to_str()) == Some(want) {
            out.push(p);
            return;
        }
        if !out.is_empty() {
            return;
        }
    }
}

/// Read-only mount trying EROFS first (local `file` on both vendor images),
/// then ext4/f2fs. Raw mount(2) needs an fstype — unlike P11's
/// `mount -o ro` auto-probe (P11 `:28`,`:36`).
fn mount_ro(src: &str, dst: &str) -> bool {
    let _ = std::fs::create_dir_all(dst);
    let c_src = match CString::new(src) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let c_dst = match CString::new(dst) {
        Ok(s) => s,
        Err(_) => return false,
    };
    for fst in ["erofs", "ext4", "f2fs"] {
        let c_fst = CString::new(fst).unwrap();
        // SAFETY: all three pointers reference live CStrings; MS_RDONLY,
        // NULL data is the standard ro-mount call.
        let rc = unsafe {
            libc::mount(
                c_src.as_ptr(),
                c_dst.as_ptr(),
                c_fst.as_ptr(),
                libc::MS_RDONLY,
                std::ptr::null(),
            )
        };
        if rc == 0 {
            info(&format!("mounted {src} ro ({fst}) on {dst}"));
            return true;
        }
    }
    info(&format!(
        "mount {src} on {dst} FAILED: {}",
        std::io::Error::last_os_error()
    ));
    false
}

fn umount(dst: &str) {
    if let Ok(c) = CString::new(dst) {
        // SAFETY: pointer references a live CString; umount(2) contract.
        unsafe {
            libc::umount(c.as_ptr());
        }
    }
}

/// Staged iff both the daemon and the module copy are in RAM (P11 `:22`:
/// `staged(){ [ -f "$RAM/aocd" ] && [ -f "$RAM/aoc_usb_driver.ko" ]; }`).
fn staged() -> bool {
    Path::new(RAM_AOCD).is_file() && Path::new(RAM_KO).is_file()
}

/// Copy the AoC runtime out of the live vendor/vendor_dlkm into RAM
/// (P11 `:27-42`). dm-mapper nodes may be late — the caller retries.
fn stage_aoc(slot: &str) -> bool {
    if staged() {
        return true;
    }
    let _ = std::fs::create_dir_all(RAM_DIR);
    if !Path::new(RAM_AOCD).is_file()
        && mount_ro(&format!("{MAP_VENDOR}{slot}"), MNT_VENDOR)
    {
        let src = Path::new(MNT_VENDOR).join(VENDOR_AOCD);
        if std::fs::copy(src, RAM_AOCD).is_ok() {
            for lib in AOC_LIBS {
                let from = Path::new(MNT_VENDOR).join(format!("lib64/{lib}.so"));
                let to = Path::new(RAM_LIB).join(format!("{lib}.so"));
                let _ = std::fs::create_dir_all(RAM_LIB);
                if std::fs::copy(&from, &to).is_err() {
                    info(&format!("stage: lib missing? {lib}"));
                }
            }
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(RAM_AOCD, std::fs::Permissions::from_mode(0o755));
            info("staged aocd + 6 libs into RAM");
        } else {
            info("stage: aocd copy FAILED (vendor not ready?)");
        }
        umount(MNT_VENDOR);
    }
    if !Path::new(RAM_KO).is_file() && mount_ro(&format!("{MAP_VDLKM}{slot}"), MNT_VDLKM) {
        let mut hits = Vec::new();
        find_named(Path::new(MNT_VDLKM), "aoc_usb_driver.ko", &mut hits);
        match hits.first() {
            Some(ko) => {
                if std::fs::copy(ko, RAM_KO).is_ok() {
                    info(&format!("staged module from {}", ko.display()));
                }
            }
            None => info("stage: aoc_usb_driver.ko NOT found on vendor_dlkm"),
        }
        umount(MNT_VDLKM);
    }
    staged()
}

/// insmod once, guarded by `/sys/module` (P11 `:69`:
/// `[ -d /sys/module/aoc_usb_driver ] || insmod ...`). Strict flags: the
/// stock module matches the running vendor kernel by construction (P11
/// proven); first-stage already provides `aoc_core` + `gvotable`.
fn ensure_module() -> bool {
    if Path::new(SYS_MODULE).exists() || is_module_loaded(MOD_NAME) {
        return true;
    }
    match load_kernel_module(Path::new(RAM_KO)) {
        Ok(()) => {
            info("aoc_usb_driver loaded");
            true
        }
        Err(e) => {
            info(&format!("aoc_usb_driver insmod FAILED: {e}"));
            false
        }
    }
}

/// Live AoC platform device dirs (`/sys/devices/platform/*.aoc`, P11 `:44-45`).
fn aoc_devices() -> Vec<PathBuf> {
    std::fs::read_dir(AOC_PLATFORM)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.ends_with(".aoc"))
                        .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `services` blob contains the `usb_control` service (P11 `:45`).
/// Pure over content (testable); live sysfs walk in [`usb_control_live`].
fn services_has_usb_control(content: &str) -> bool {
    content.lines().any(|l| l.trim() == USB_CONTROL)
}

/// Live vote check: any `services` file under the AoC devices lists
/// `usb_control` (P11 `:45`,`:70`).
fn usb_control_live() -> bool {
    for dev in aoc_devices() {
        let mut stack = vec![dev];
        while let Some(dir) = stack.pop() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.file_name().and_then(|n| n.to_str()) == Some(SERVICES_NODE) {
                    let c = std::fs::read_to_string(&p).unwrap_or_default();
                    if services_has_usb_control(&c) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// `verify_aoc_responsive` content check (P11 `:44`). Pure (testable).
fn is_responsive_state(content: &str) -> bool {
    content.trim() == RESPONSIVE
}

/// Live responsiveness gate (P11 `:72`).
fn aoc_responsive() -> bool {
    aoc_devices().iter().any(|dev| {
        is_responsive_state(&std::fs::read_to_string(dev.join(VERIFY_NODE)).unwrap_or_default())
    })
}

/// SIGKILL a previous `aocd` so instances never pile up (P11 `:73`:
/// `kill $(pidof aocd)`). /proc scan — no fork.
fn kill_aocd() {
    let procs = match std::fs::read_dir("/proc") {
        Ok(r) => r,
        Err(_) => return,
    };
    for entry in procs.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let pid: i32 = match name.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let comm =
            std::fs::read_to_string(entry.path().join("comm")).unwrap_or_default();
        if comm.trim() == "aocd" {
            // SAFETY: pid comes from our own /proc scan; SIGKILL a same-name
            // daemon is the documented P11 relaunch step.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
            info(&format!("killed stale aocd (pid {pid})"));
        }
    }
}

/// Launch a fresh `aocd` detached (P11 `:74`:
/// `LD_LIBRARY_PATH="$RAM/lib":/system/lib64 "$RAM/aocd" &`).
fn spawn_aocd() {
    let libs = format!("{RAM_LIB}:/system/lib64");
    match std::process::Command::new(RAM_AOCD)
        .env("LD_LIBRARY_PATH", libs)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(_) => info("spawned fresh aocd"),
        Err(e) => info(&format!("aocd spawn FAILED: {e}")),
    }
}

/// Controller role-switch `role` file: prefer the DWC3 controller entry
/// (`a200000.usb3-role-switch`, live device log), else first sorted entry
/// (zuma.rs `pick_role_entry` pattern). Pure over names (testable); live
/// readdir in [`find_data_role`].
fn pick_role_entry(entries: &[String]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    entries
        .iter()
        .find(|n| n.contains(ROLE_PREFER))
        .or_else(|| entries.first())
        .cloned()
}

/// Live data-role control file.
fn find_data_role() -> Option<PathBuf> {
    let mut names: Vec<String> = std::fs::read_dir(USB_ROLE_CLASS)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    pick_role_entry(&names).map(|n| Path::new(USB_ROLE_CLASS).join(n).join("role"))
}

/// Current controller data role (`host`/`device`/…).
fn read_data_role() -> String {
    find_data_role()
        .map(|p| read_trim(&p.to_string_lossy()))
        .unwrap_or_default()
}

/// Source psy `online` file: exact live name wins, then any
/// `*tcpm-source*`, then any `*source*` (name drifts across spins; the
/// kernel carries `tcpm-source-psy`×1). Pure over names (testable).
fn pick_source_psy(entries: &[String]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    entries
        .iter()
        .find(|n| *n == SOURCE_PSY)
        .or_else(|| entries.iter().find(|n| n.contains("tcpm-source")))
        .or_else(|| entries.iter().find(|n| n.contains("source")))
        .cloned()
}

/// Live source psy `online` file.
fn find_source_psy() -> Option<PathBuf> {
    let names: Vec<String> = std::fs::read_dir(PSY_CLASS)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    pick_source_psy(&names).map(|n| Path::new(PSY_CLASS).join(n).join("online"))
}

/// TCPC-sourced VBUS state (`1` = sourcing).
fn read_source_online() -> String {
    find_source_psy()
        .map(|p| read_trim(&p.to_string_lossy()))
        .unwrap_or_default()
}

/// Host path ACTIVE only on BOTH signals — never the role alone: a bare
/// `role=host` readback with no sourced VBUS is not host mode.
fn host_active() -> bool {
    read_data_role() == "host" && read_source_online() == "1"
}

/// First removable USB disk (`sdX1` preferred, else whole disk) — zuma.rs
/// `pick_otg_disk` pattern for the P11 `usb_disk()` (P11 `:49-58`).
/// Pure core over candidates (testable); live sysfs walk in [`pick_otg_disk`].
fn choose_otg_disk(cands: &[(&str, bool, bool, bool)]) -> Option<String> {
    let mut names: Vec<&str> = cands
        .iter()
        .filter(|(_, removable, has_usb, _)| *removable && *has_usb)
        .map(|(n, _, _, _)| *n)
        .collect();
    names.sort();
    names.first().map(|n| {
        let has_part = cands
            .iter()
            .find(|(m, _, _, _)| m == n)
            .map(|(_, _, _, p)| *p)
            .unwrap_or(false);
        if has_part {
            format!("/dev/block/{n}1")
        } else {
            format!("/dev/block/{n}")
        }
    })
}

/// Live removable-USB-disk pick.
fn pick_otg_disk() -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir("/sys/block")
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| {
                    n.len() == 3
                        && n.starts_with("sd")
                        && n.as_bytes().get(2).copied().map(|c| c.is_ascii_lowercase()).unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    let mut cands: Vec<(String, bool, bool, bool)> = Vec::new();
    for n in &names {
        let dir = Path::new("/sys/block").join(n);
        let removable = read_trim(&dir.join("removable").to_string_lossy()) == "1";
        let dev = std::fs::read_link(dir.join("device"))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let has_usb = dev.contains("usb");
        let has_part = Path::new(&format!("/dev/block/{n}1")).exists();
        cands.push((n.clone(), removable, has_usb, has_part));
    }
    let refs: Vec<(&str, bool, bool, bool)> = cands
        .iter()
        .map(|(n, r, u, p)| (n.as_str(), *r, *u, *p))
        .collect();
    choose_otg_disk(&refs)
}

/// Keep `/dev/block/otg-usb` (`/usb_otg`) on the current OTG disk; relink
/// only on change (P11 `:78-80`; zuma.rs `ensure_otg_symlink` pattern).
fn ensure_otg_symlink() {
    let target = match pick_otg_disk() {
        Some(t) => t,
        None => return,
    };
    let cur = std::fs::read_link(OTG_USB_LINK)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    if cur == target {
        return;
    }
    let _ = std::fs::remove_file(OTG_USB_LINK);
    match std::os::unix::fs::symlink(&target, OTG_USB_LINK) {
        Ok(()) => info(&format!("otg-usb -> {target}")),
        Err(e) => info(&format!("otg-usb symlink FAILED: {e}")),
    }
}

/// Single vote-keepalive step (P11 `:70-77`): `usb_control` present = vote
/// live (log the transition once); responsive-but-no-service = the flaky
/// startup → kill + fresh instance + settle wait, then recheck. `usb_control`
/// persists once up, so relaunching stops then.
fn keep_vote_alive(voted: &mut bool) {
    if usb_control_live() {
        if !*voted {
            *voted = true;
            info("aocd up; AOC vote live");
        }
        return;
    }
    *voted = false;
    if aoc_responsive() {
        kill_aocd();
        spawn_aocd();
        sleep(Duration::from_secs(AOCD_SETTLE_SECS));
        if usb_control_live() {
            *voted = true;
            info("aocd up; AOC vote live");
        } else {
            info("aocd relaunch: usb_control not up yet (flaky startup, retrying)");
        }
    }
}

/// otg-patch (oneshot, service `otg_enable`): stage the stock AoC runtime
/// into RAM → insmod once → bounded `aocd` relaunch until the `usb_control`
/// vote is live → `sys.usb.patch_dwc3=1` (gates `otg_auto`). `0` + `Err`
/// otherwise: device mode survives, adb safe.
pub fn run_otg_patch() -> Result<(), String> {
    info("starting OTG patch routine (malibu: stage AoC runtime + vote)");
    // ONLY 6.12 exists on malibu; anything else is unverified → fail closed.
    match crate::ko_picker::detect_kernel_env() {
        Ok(e) if (e.major, e.minor) == (6, 12) => {
            info(&format!("kernel {}.{} (malibu 6.12-only path)", e.major, e.minor));
        }
        Ok(e) => {
            info(&format!(
                "kernel {}.{}: malibu host path UNVERIFIED here, staying in device mode",
                e.major, e.minor
            ));
            let _ = set_prop("sys.usb.patch_dwc3", "0");
            return Err("malibu otg: unsupported kernel (needs 6.12 role-switch)".into());
        }
        Err(e) => {
            info(&format!("detect_kernel_env failed ({e}), staying in device mode"));
            let _ = set_prop("sys.usb.patch_dwc3", "0");
            return Err("malibu otg: kernel env unknown".into());
        }
    }

    // 1. Stage aocd + 6 libs + module into tmpfs (dm-mapper nodes appear
    // late — retry bounded; the daemon retries forever afterwards).
    let slot = slot_suffix();
    info(&format!("slot suffix: {slot:?}"));
    let mut ok = false;
    for attempt in 0..STAGE_ATTEMPTS {
        if stage_aoc(&slot) {
            ok = true;
            break;
        }
        info(&format!(
            "staging pending (attempt {}/{STAGE_ATTEMPTS}, vendor nodes late?)",
            attempt + 1
        ));
        sleep(Duration::from_secs(TICK_SECS));
    }
    if !ok {
        info("staging FAILED: staying in device mode");
        let _ = set_prop("sys.usb.patch_dwc3", "0");
        return Err("malibu otg: staging failed".into());
    }
    info("staged aocd + 6 libs + module into RAM");

    // 2. Module into the kernel (once; first-stage already loaded aoc_core
    // + gvotable, its dep providers).
    if !ensure_module() {
        let _ = set_prop("sys.usb.patch_dwc3", "0");
        return Err("malibu otg: aoc_usb_driver load failed".into());
    }

    // 3. The vote: relaunch flaky aocd until usb_control appears (bounded —
    // the daemon keeps this alive afterwards).
    let mut voted = usb_control_live();
    for _ in 0..AOCD_ATTEMPTS {
        keep_vote_alive(&mut voted);
        if voted {
            break;
        }
        sleep(Duration::from_secs(TICK_SECS));
    }
    if !voted {
        info("AoC vote NOT live after bounded relaunches: staying in device mode");
        let _ = set_prop("sys.usb.patch_dwc3", "0");
        return Err("malibu otg: aoc vote not live".into());
    }

    // 4. Report the host predicate (role AND sourced VBUS — never role
    // alone). At boot with no accessory attached this is normally
    // device/offline; the live vote above is what serves a later attach,
    // so it only informs the log here.
    info(&format!(
        "vote live: data_role={:?} source_online={:?} host_active={}",
        read_data_role(),
        read_source_online(),
        host_active()
    ));
    // No i2c TCPC patch by design (SPMI bus — see module docs).
    set_prop("sys.usb.patch_dwc3", "1").map_err(|e| e.to_string())?;
    info("OTG patch routine finished, sys.usb.patch_dwc3=1");
    Ok(())
}

/// otg-auto (service `otg_auto`): supervisor that keeps the AoC vote alive
/// and the `otg-usb` symlink on the current disk. Never exits (an exiting
/// non-oneshot service would respawn-loop). Never touches UDC/gadget/role/
/// voter state — TCPC + role-sw negotiate modes on their own, so device
/// mode (adb) always survives.
pub fn run_otg_auto() -> ! {
    info("starting (malibu supervisor: keep AoC vote alive + otg-usb symlink)");
    let slot = slot_suffix();
    let mut voted = false;
    loop {
        if !staged() && stage_aoc(&slot) {
            info("staged aocd + 6 libs + module into RAM");
        }
        if staged() {
            if !ensure_module() {
                info("supervisor: module not loaded yet (retrying)");
            }
            keep_vote_alive(&mut voted);
            if pick_otg_disk().is_some() {
                info(&format!(
                    "disk present: data_role={:?} source_online={:?} host_active={}",
                    read_data_role(),
                    read_source_online(),
                    host_active()
                ));
            }
            ensure_otg_symlink();
        }
        sleep(Duration::from_secs(TICK_SECS));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_suffix_prefers_prop() {
        assert_eq!(parse_slot_suffix("_b", ""), "_b");
        assert_eq!(parse_slot_suffix(" _a\n", "androidboot.slot_suffix = \"_b\""), "_a");
    }

    #[test]
    fn slot_suffix_falls_back_to_bootconfig() {
        let bc = "androidboot.serialno = \"abc\"\nandroidboot.slot_suffix = \"_a\"\n";
        assert_eq!(parse_slot_suffix("", bc), "_a");
        // Spaces around '=' and quotes are tolerated.
        assert_eq!(parse_slot_suffix("", "androidboot.slot_suffix=_b"), "_b");
        // Empty everywhere reads as no slot (fail closed, mapper has no suffix).
        assert_eq!(parse_slot_suffix("", ""), "");
        assert_eq!(parse_slot_suffix("", "unrelated = \"x\""), "");
    }

    #[test]
    fn services_vote_parses() {
        // Real shape: one service per line in the sysfs blob.
        assert!(services_has_usb_control("aoc_control\nusb_control\n"));
        assert!(!services_has_usb_control("aoc_control\naoc_audio\n"));
        assert!(!services_has_usb_control(""));
        // Substring hits must not count (usb_control_ping is not the vote).
        assert!(!services_has_usb_control("usb_control_ping\n"));
    }

    #[test]
    fn responsive_gate_parses() {
        assert!(is_responsive_state("responsive\n"));
        assert!(!is_responsive_state("unresponsive\n"));
        assert!(!is_responsive_state(""));
    }

    #[test]
    fn role_entry_prefers_controller() {
        // Live kodiak entries: controller switch wins over the repeater one.
        let two = vec![
            "i2c-8-003e-eusb2-repeater-role-switch".to_string(),
            "a200000.usb3-role-switch".to_string(),
        ];
        assert_eq!(
            pick_role_entry(&two),
            Some("a200000.usb3-role-switch".to_string())
        );
        // No controller entry: first sorted entry (zuma-compatible fallback).
        let one = vec!["other-switch".to_string()];
        assert_eq!(pick_role_entry(&one), Some("other-switch".to_string()));
        let empty: Vec<String> = Vec::new();
        assert_eq!(pick_role_entry(&empty), None);
    }

    #[test]
    fn source_psy_prefers_live_name() {
        // Live kodiak psy set: exact TCPC source name wins.
        let names = vec![
            "battery".to_string(),
            "usb".to_string(),
            SOURCE_PSY.to_string(),
        ];
        assert_eq!(pick_source_psy(&names), Some(SOURCE_PSY.to_string()));
        // Renamed spin: any tcpm-source fallback still finds it.
        let renamed = vec![
            "usb".to_string(),
            "tcpm-source-psy-spmi-foo".to_string(),
        ];
        assert_eq!(
            pick_source_psy(&renamed),
            Some("tcpm-source-psy-spmi-foo".to_string())
        );
        let none = vec!["battery".to_string(), "usb".to_string()];
        assert_eq!(pick_source_psy(&none), None);
    }

    #[test]
    fn disk_pick_prefers_first_partition() {
        // (name, removable, device-link-via-usb, has-partition).
        let cands = [
            ("sdb", true, true, false),
            ("sda", true, true, true),
            ("sdc", false, true, true),
            ("sdd", true, false, true),
        ];
        // sda wins (removable + usb); partition preferred.
        assert_eq!(
            choose_otg_disk(&cands),
            Some("/dev/block/sda1".to_string())
        );
        // Whole disk when no partition node exists yet.
        assert_eq!(
            choose_otg_disk(&[("sdb", true, true, false)]),
            Some("/dev/block/sdb".to_string())
        );
        // Non-removable / non-USB devices never match.
        assert_eq!(choose_otg_disk(&[("sdc", false, true, true)]), None);
        assert_eq!(choose_otg_disk(&[("sdd", true, false, true)]), None);
        let empty: [(&str, bool, bool, bool); 0] = [];
        assert_eq!(choose_otg_disk(&empty), None);
    }
}
