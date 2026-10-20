//! fox-hsinstall CLI: thin arg parse, everything else in run::run.

use hs_install::run;

fn usage() -> ! {
    eprintln!(
        "usage: fox-hsinstall --zip PATH [--status-fd N] [--slot a|b]\n                          [--sysroot DIR] [--tmpdir DIR] [--threads N]"
    );
    // Distinct from the fallback codes 2/5: a broken invocation must be
    // loud, never masquerade as "not full OTA".
    std::process::exit(6);
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let zip = run::flag(&args, "--zip").unwrap_or_else(|| usage());
    let status_fd = run::flag(&args, "--status-fd").and_then(|s| s.parse::<i32>().ok());
    let slot = run::flag(&args, "--slot").and_then(|s| s.chars().next());
    if let Some(s) = slot {
        if s != 'a' && s != 'b' {
            usage();
        }
    }
    let sysroot = run::flag(&args, "--sysroot").unwrap_or_else(|| "/system/bin".to_string());
    let tmpdir = run::flag(&args, "--tmpdir").unwrap_or_else(|| "/tmp/hs_backup".to_string());
    let threads = run::flag(&args, "--threads")
        .map(|s| s.parse().unwrap_or_else(|_| usage()))
        .unwrap_or(8);
    let code = run::run(run::Args {
        zip,
        status_fd,
        slot,
        sysroot,
        tmpdir,
        threads,
    });
    std::process::ExitCode::from(code as u8)
}
