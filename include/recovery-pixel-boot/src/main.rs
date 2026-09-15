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
