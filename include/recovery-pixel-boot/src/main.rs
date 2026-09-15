//! recovery-pixel-boot — static Rust replacement for the Pixel recovery
//! shell scripts (runatinit.sh / runatboot.sh / otg_patch.sh / otg_auto_v3.sh).
//!
//! Multicall binary (same pattern as recovery-tensor-daemon):
//!   recovery-pixel-boot init       # early-init device identity (was runatinit.sh)
//!   recovery-pixel-boot boot       # post-GUI boot setup (was runatboot.sh, via twrp.cpp)
//!   recovery-pixel-boot otg-patch  # OTG shim inject (was otg_patch.sh, service otg_enable)
//!   recovery-pixel-boot otg-auto   # VBUS auto-switch daemon (was otg_auto_v3.sh, service otg_auto)
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

use std::process::ExitCode;

fn usage() -> ! {
    eprintln!(
        "Usage:\n\
         \x20 recovery-pixel-boot init\n\
         \x20 recovery-pixel-boot boot\n\
         \x20 recovery-pixel-boot otg-patch\n\
         \x20 recovery-pixel-boot otg-auto\n"
    );
    std::process::exit(1);
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        usage();
    }
    match args[1].as_str() {
        "init" => match init::run_init() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("recovery-pixel-boot init: {e}");
                ExitCode::FAILURE
            }
        },
        "boot" => match boot::run_boot() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("recovery-pixel-boot boot: {e}");
                ExitCode::FAILURE
            }
        },
        "otg-patch" => match otg::run_otg_patch() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("recovery-pixel-boot otg-patch: {e}");
                ExitCode::FAILURE
            }
        },
        "otg-auto" => {
            // Diverges under normal operation; the `!` coerces to ExitCode.
            otg::run_otg_auto()
        }
        _ => usage(),
    }
}
