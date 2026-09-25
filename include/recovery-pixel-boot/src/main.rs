//! recovery-pixel-boot — static Rust replacement for the Pixel recovery
//! shell scripts.
//!
//! Multicall binary (same pattern as recovery-tensor-daemon):
//!   recovery-pixel-boot init        # early-init device identity
//!   recovery-pixel-boot boot        # vendor boot stage (touch, fw, magisk)
//!   recovery-pixel-boot otg-patch   # OTG shim inject (service otg_enable)
//!   recovery-pixel-boot otg-auto    # VBUS auto-switch daemon (service otg_auto)
//!   recovery-pixel-boot setup-temp  # thermal zone symlink (on init exec)
//!   recovery-pixel-boot torch on|off# LM3644 flashlight (Fox OF_FL_PATH hook)
//!
//! Only std + libc. Exit 0 ok / skip (monolithic kernel), 1 fatal.

mod boot;
mod config;
mod i2c;
mod init;
mod ko_picker;
mod otg;
mod props;
mod stage;
mod temp;
mod torch;

use std::process::ExitCode;

/// Unpack-completion markers dropped by recovery-init-stub (and by the
/// init.cpp legacy path) once the LGZ cluster is unpacked and the tree is
/// final. Either copy suffices (`/system` may be over-mounted later, the
/// root copy always stays visible).
const MARKER_ROOT: &str = "lgz_complite";
const MARKER_ETC: &str = "system/etc/lgz_complite";

/// True when at least one unpack-completion marker exists under `root`.
/// Split out for testability; production passes `/`.
fn markers_present_under(root: &std::path::Path) -> bool {
    root.join(MARKER_ROOT).exists() || root.join(MARKER_ETC).exists()
}

fn markers_present() -> bool {
    markers_present_under(std::path::Path::new("/"))
}

fn run_simple(res: Result<(), String>, name: &str) -> ExitCode {
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("recovery-pixel-boot {name}: {e}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> ! {
    eprintln!(
        "Usage:\n\
         \x20 recovery-pixel-boot init\n\
         \x20 recovery-pixel-boot boot\n\
         \x20 recovery-pixel-boot otg-patch\n\
         \x20 recovery-pixel-boot otg-auto\n\
         \x20 recovery-pixel-boot setup-temp\n\
         \x20 recovery-pixel-boot torch on|off\n"
    );
    std::process::exit(1);
}

fn main() -> ExitCode {
    // Guard: the tree is usable only after the LGZ cluster was unpacked
    // (stub or legacy path drops the markers first). Without them this
    // binary must not touch anything — silent success, exit 0.
    if !markers_present() {
        return ExitCode::SUCCESS;
    }
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        usage();
    }
    match args[1].as_str() {
        "init" => run_simple(init::run_init(), "init"),
        "boot" => run_simple(boot::run_boot(), "boot"),
        "otg-patch" => run_simple(otg::run_otg_patch(), "otg-patch"),
        "setup-temp" => run_simple(temp::run_setup_temp(), "setup-temp"),
        "torch" => {
            if args.len() < 3 {
                usage();
            }
            run_simple(torch::run_torch(&args[2]), "torch")
        }
        "otg-auto" => {
            // Diverges under normal operation; the `!` coerces to ExitCode.
            otg::run_otg_auto()
        }
        _ => usage(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpack_markers_gate() {
        let d = std::env::temp_dir().join(format!("fox_test_markers_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        // No markers: gate closed.
        assert!(!markers_present_under(&d));
        // Either marker alone opens the gate.
        std::fs::write(d.join(MARKER_ROOT), b"lgz_cluster unpacked\n").unwrap();
        assert!(markers_present_under(&d));
        std::fs::remove_file(d.join(MARKER_ROOT)).unwrap();
        assert!(!markers_present_under(&d));
        std::fs::create_dir_all(d.join("system/etc")).unwrap();
        std::fs::write(d.join(MARKER_ETC), b"lgz_cluster unpacked\n").unwrap();
        assert!(markers_present_under(&d));
        let _ = std::fs::remove_dir_all(&d);
    }
}
