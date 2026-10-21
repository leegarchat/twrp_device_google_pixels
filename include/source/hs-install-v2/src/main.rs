//! fox-hsinstall CLI: thin arg parse, everything else in run::run.

use hs_install::run;

fn usage() -> ! {
    eprintln!(
        "usage: fox-hsinstall --zip PATH [--status-fd N] [--slot a|b] [--booted-slot a|b]\n                          [--sysroot DIR] [--tmpdir DIR] [--threads N] [--readback]"
    );
    // Distinct from the fallback codes 2/5: a broken invocation must be
    // loud, never masquerade as "not full OTA".
    std::process::exit(hs_install::EXIT_USAGE);
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
    let booted_slot = run::flag(&args, "--booted-slot").and_then(|s| s.chars().next());
    if let Some(b) = booted_slot {
        if b != 'a' && b != 'b' {
            usage();
        }
    }
    let sysroot = run::flag(&args, "--sysroot").unwrap_or_else(|| "/system/bin".to_string());
    let tmpdir = run::flag(&args, "--tmpdir").unwrap_or_else(|| "/tmp/hs_backup".to_string());
    let readback = args.iter().any(|a| a == "--readback");
    // Worker budget (peridot parity): min(ncpu,8), additionally capped by
    // free RAM (512MiB reserve + 96MiB per worker). --threads overrides.
    let auto_threads = || -> usize {
        let mut w = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8);
        w = w.min(8).max(1);
        if let Ok(mem) = std::fs::read_to_string("/proc/meminfo") {
            for line in mem.lines() {
                if let Some(rest) = line.strip_prefix("MemAvailable:") {
                    if let Some(kb) = rest
                        .split_whitespace()
                        .next()
                        .and_then(|s| s.parse::<u64>().ok())
                    {
                        let cap = kb.saturating_div(1024).saturating_sub(512) / 96;
                        if cap >= 1 {
                            w = w.min(cap as usize);
                        } else {
                            w = 1;
                        }
                    }
                    break;
                }
            }
        }
        w.max(1)
    };
    let threads = run::flag(&args, "--threads")
        .map(|s| {
            let n: usize = s.parse().unwrap_or_else(|_| usage());
            n.clamp(1, 16)
        })
        .unwrap_or_else(auto_threads);
    let code = run::run(run::Args {
        zip,
        status_fd,
        slot,
        booted_slot,
        sysroot,
        tmpdir,
        threads,
        readback,
    });
    std::process::ExitCode::from(code as u8)
}
