//! stage — the single choke point for calling out to pixelrunatboot.sh.
//!
//! The Rust engine never forks business-logic binaries directly. Every
//! external tool (resetprop, bootctl, siw/iw, unzip, mount, setenforce)
//! runs inside a pixelrunatboot.sh stage. Protocol: argv stage + args,
//! results on stdout, diagnostics in /tmp/recovery.log, exit 0 = success.

use std::process::Command;

const SH: &str = "/system/bin/sh";
const SCRIPT: &str = "/system/bin/pixelrunatboot.sh";

/// Run one stage, return trimmed stdout. Err on spawn failure or rc != 0.
pub fn run_stage(stage: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(SH)
        .arg(SCRIPT)
        .arg(stage)
        .args(args)
        .output()
        .map_err(|e| format!("stage {stage}: spawn failed: {e}"))?;
    if !out.status.success() {
        return Err(format!("stage {stage} failed: {:?}", out.status.code()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Pure-Rust mountpoint check via /proc/mounts (no `mountpoint` fork).
pub fn is_mounted(mnt: &str) -> bool {
    let content = match std::fs::read_to_string("/proc/mounts") {
        Ok(c) => c,
        Err(_) => return false,
    };
    mounts_contain(&content, mnt)
}

pub fn mounts_contain(mounts: &str, mnt: &str) -> bool {
    mounts
        .lines()
        .any(|l| l.split_whitespace().nth(1) == Some(mnt))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mounts_parsing() {
        let sample = "tmpfs /dev tmpfs rw,seclabel,nosuid 0 0\n\
                      /dev/block/dm-8 /vendor/firmware erofs ro 0 0\n";
        assert!(mounts_contain(sample, "/dev"));
        assert!(mounts_contain(sample, "/vendor/firmware"));
        assert!(!mounts_contain(sample, "/vendor"));
        assert!(!mounts_contain(sample, "/data"));
    }

    #[test]
    fn missing_stage_is_err() {
        // No such stage -> script exits 2 (or sh missing on host -> spawn err).
        assert!(run_stage("definitely-not-a-stage", &[]).is_err());
    }
}
