//! recovery-tensor-daemon: unified FBE-decryption daemon for Pixel (Tensor) recovery.
//!
//! Merges two formerly separate C/C++ daemons into one multicall binary:
//!   * `storageproxy` — Trusty secure-storage proxy (TIPC + RPMB over UFS/SG_IO).
//!   * `weaver` — Titan M (GSC/Citadel) proxy publishing
//!     `android.hardware.weaver.IWeaver/default` over Binder.
//!   * `run`          — both services in parallel threads with joint coordination.
//!
//! Only `libc` is used as an external dependency so the binary stays
//! dependency-free for plain cargo builds; the Binder/AIDL glue behind the
//! `binder` cargo feature is enabled by the Soong build (see Android.bp).

// `logi!`/`loge!`/`logd!` are #[macro_export]ed at the crate root.
mod common;
mod storageproxy;
mod weaver;

use std::process::ExitCode;

const DEFAULT_GSC_DEV: &str = "/dev/gsc0";

fn usage() -> ! {
    eprintln!(
        "Usage:\n\
         \x20 recovery-tensor-daemon storageproxy -d <trusty_dev> [-r <rpmb_dev>] -p <data_path>\n\
         \x20 recovery-tensor-daemon weaver [gsc_dev]            (default: {DEFAULT_GSC_DEV})\n\
         \x20 recovery-tensor-daemon run -d <trusty_dev> [-r <rpmb_dev>] -p <data_path> [gsc_dev]\n\
         \n\
         storageproxy proxies com.android.trusty.storage.proxy between Trusty\n\
         (TIPC) and the UFS RPMB well-known LUN (SCSI generic SG_IO).\n\
         If -r is omitted, the RPMB sg node is auto-discovered and cached\n\
         in /tmp/.rpmb_sg_dev.\n\
         \n\
         weaver proxies the Titan M (GSC/Citadel) secure element and registers\n\
         android.hardware.weaver.IWeaver/default on Binder for CE FBE unlock.\n\
         \n\
         run starts both services in parallel threads."
    );
    std::process::exit(1);
}

/// Parsed storageproxy options shared by `storageproxy` and `run`.
struct StorageArgs {
    trusty_dev: String,
    rpmb_dev: Option<String>,
    data_path: String,
}

/// Parses `-d <trusty> [-r <rpmb>] -p <path>` from an arg slice.
fn parse_storage_opts(args: &[String], sub: &str) -> StorageArgs {
    let mut trusty: Option<String> = None;
    let mut rpmb: Option<String> = None;
    let mut path: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-d" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("{sub}: -d requires a trusty device argument");
                    usage();
                }
                trusty = Some(args[i].clone());
            }
            "-r" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("{sub}: -r requires an rpmb device argument");
                    usage();
                }
                rpmb = Some(args[i].clone());
            }
            "-p" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("{sub}: -p requires a data path argument");
                    usage();
                }
                path = Some(args[i].clone());
            }
            "-h" | "--help" => usage(),
            other => {
                eprintln!("{sub}: unknown option '{other}'");
                usage();
            }
        }
        i += 1;
    }
    let (Some(trusty_dev), Some(data_path)) = (trusty, path) else {
        eprintln!("{sub}: both -d <trusty_dev> and -p <data_path> are required");
        usage();
    };
    StorageArgs { trusty_dev, rpmb_dev: rpmb, data_path }
}

fn cmd_storageproxy(args: &[String]) -> ExitCode {
    let opts = parse_storage_opts(args, "storageproxy");
    // Restrict file creation mask like the original C daemon (0700 dirs, 0600 files).
    // Safety: umask has no preconditions.
    unsafe { libc::umask(0o077) };
    match storageproxy::run(&opts.trusty_dev, opts.rpmb_dev.as_deref(), &opts.data_path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            loge!("sp", "fatal: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_weaver(args: &[String]) -> ExitCode {
    if args.len() > 1 {
        eprintln!("weaver: too many arguments (expected at most one gsc device)");
        usage();
    }
    if args.first().is_some_and(|a| a == "-h" || a == "--help") {
        usage();
    }
    let dev = args.first().map(String::as_str).unwrap_or(DEFAULT_GSC_DEV);
    // Safety: umask has no preconditions.
    unsafe { libc::umask(0o077) };
    weaver::run(dev);
}

fn cmd_run(args: &[String]) -> ExitCode {
    // `run` takes the storageproxy flags plus an optional trailing positional
    // gsc device: run -d <t> [-r <r>] -p <p> [gsc_dev].
    let (opts_args, gsc_dev) = split_run_args(args);
    let opts = parse_storage_opts(&opts_args, "run");
    // Safety: umask has no preconditions.
    unsafe { libc::umask(0o077) };

    logi!("main", "starting storageproxy + weaver");

    let sp_handle = std::thread::Builder::new()
        .name("storageproxy".to_string())
        .spawn(move || {
            if let Err(e) = storageproxy::run(&opts.trusty_dev, opts.rpmb_dev.as_deref(), &opts.data_path) {
                loge!("sp", "fatal: {e}");
                std::process::exit(1);
            }
        });
    match sp_handle {
        Ok(_handle) => {
            // Diverges: joins the Binder pool (Soong) or supervises the GSC
            // link forever; the `!` return coerces to the match type.
            weaver::run(&gsc_dev);
        }
        Err(e) => {
            loge!("main", "failed to spawn storageproxy thread: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Splits `run` args into (flag args, gsc_dev): a trailing non-flag token that
/// is not the value of -d/-r/-p is treated as the gsc device path.
fn split_run_args(args: &[String]) -> (Vec<String>, String) {
    let mut flags: Vec<String> = Vec::with_capacity(args.len());
    let mut positional: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if (a == "-d" || a == "-r" || a == "-p") && i + 1 < args.len() {
            flags.push(args[i].clone());
            flags.push(args[i + 1].clone());
            i += 2;
        } else if a == "-h" || a == "--help" {
            usage();
        } else if a.starts_with('-') {
            eprintln!("run: unknown option '{a}'");
            usage();
        } else {
            positional.push(args[i].clone());
            i += 1;
        }
    }
    if positional.len() > 1 {
        eprintln!("run: too many positional arguments (expected at most one gsc device)");
        usage();
    }
    let gsc = positional.into_iter().next().unwrap_or_else(|| DEFAULT_GSC_DEV.to_string());
    (flags, gsc)
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() < 2 {
        usage();
    }
    match argv[1].as_str() {
        "storageproxy" => cmd_storageproxy(&argv[2..]),
        "weaver" => cmd_weaver(&argv[2..]),
        "run" => cmd_run(&argv[2..]),
        "-h" | "--help" | "help" => usage(),
        other => {
            eprintln!("unknown subcommand '{other}'");
            usage();
        }
    }
}
