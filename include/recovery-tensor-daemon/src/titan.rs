//! Titan M secure-element user-secret wipe (`titan-wipe` subcommand).
//!
//! Mirrors the stock factory-reset `WipeTitanM` (AOSP
//! `hardware/google/pixel/recovery/recovery_ui.cpp`, same call sequence the
//! fastbootd `doOemSpecificErase` uses): erase every user secret the chip
//! holds via `NUKE_FROM_ORBIT`, up to 5 attempts. There is no per-generation
//! branch: M1/Citadel, M2 (Dauntless) and M3 share the stable Nugget protocol
//! (`APP_ID_NUGGET` 0x00, param 0x0005, `ERASE_CONFIRMATION` magic); the
//! transport already selects `/dev/citadel0` vs `/dev/gsc0` at open time.
//! The `--check` read of the firmware version string is the generation
//! probe (Dauntless-prefixed = M2 family); it gates nothing, it only logs.
//!
//! `/dev/gsc0` serves one caller at a time, so the TWRP caller stops the
//! `recovery_weaver` service before running `--erase` and restarts it after.

use std::process::ExitCode;

use crate::weaver::gsc::{GscDevice, APP_SUCCESS};

/// Nugget app id (all GSC generations). Verified against
/// `external/nos/host/generic/nugget/include/application.h`.
const APP_ID_NUGGET: u8 = 0x00;
/// Nugget firmware-version query. Same source.
const NUGGET_PARAM_VERSION: u16 = 0x0000;
/// Nuke-everything command. Same source (`app_nugget.h`).
const NUGGET_PARAM_NUKE_FROM_ORBIT: u16 = 0x0005;
/// Nuke confirmation magic, little-endian on the wire. Same source.
const ERASE_CONFIRMATION: u32 = 0xc05fefee;

/// Stock gives up after five tries.
const ERASE_ATTEMPTS: u32 = 5;

fn check(dev: &str) -> ExitCode {
    let gsc = match GscDevice::open(dev) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("titan-wipe: cannot open {dev}: {e}");
            return ExitCode::FAILURE;
        }
    };
    match gsc.nos_call(APP_ID_NUGGET, NUGGET_PARAM_VERSION, &[], 512) {
        Ok((reply, status)) => {
            if status != APP_SUCCESS {
                eprintln!("titan-wipe: version read failed, status {status}");
                return ExitCode::FAILURE;
            }
            let version = String::from_utf8_lossy(&reply);
            println!("titan-wipe: Titan M firmware: {version}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("titan-wipe: version read failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn erase(dev: &str) -> ExitCode {
    let magic = ERASE_CONFIRMATION.to_le_bytes();
    for attempt in 1..=ERASE_ATTEMPTS {
        let gsc = match GscDevice::open(dev) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("titan-wipe: attempt {attempt}: cannot open {dev}: {e}");
                continue;
            }
        };
        match gsc.nos_call(APP_ID_NUGGET, NUGGET_PARAM_NUKE_FROM_ORBIT, &magic, 0) {
            Ok((_, status)) => {
                if status == APP_SUCCESS {
                    println!("titan-wipe: Titan M wipe successful");
                    return ExitCode::SUCCESS;
                }
                eprintln!("titan-wipe: attempt {attempt}: status {status}");
            }
            Err(e) => {
                eprintln!("titan-wipe: attempt {attempt}: call failed: {e}");
            }
        }
    }
    eprintln!("titan-wipe: Titan M user data wipe failed");
    ExitCode::FAILURE
}

/// `titan-wipe [--check|--erase] [gsc_dev]`.
pub fn run(args: &[String]) -> ExitCode {
    let mut check_only = false;
    let mut do_erase = false;
    let mut dev: Option<&str> = None;
    for a in args {
        match a.as_str() {
            "--check" => check_only = true,
            "--erase" => do_erase = true,
            "-h" | "--help" => {
                eprintln!("usage: recovery-tensor-daemon titan-wipe [--check|--erase] [gsc_dev]");
                std::process::exit(2);
            }
            other if other.starts_with('-') => {
                eprintln!("titan-wipe: unknown option '{other}'");
                std::process::exit(2);
            }
            d => {
                if dev.is_some() {
                    eprintln!("titan-wipe: too many positional arguments");
                    std::process::exit(2);
                }
                dev = Some(d);
            }
        }
    }
    if check_only == do_erase {
        eprintln!("usage: recovery-tensor-daemon titan-wipe [--check|--erase] [gsc_dev]");
        std::process::exit(2);
    }
    let dev = dev.unwrap_or(crate::DEFAULT_GSC_DEV);
    // Safety: umask has no preconditions.
    unsafe { libc::umask(0o077) };
    if check_only {
        check(dev)
    } else {
        erase(dev)
    }
}
