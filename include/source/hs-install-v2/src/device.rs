//! Device identity: props via getprop, slot math. No hardcoded devices.

use std::process::Command;

pub fn getprop(name: &str) -> Result<String, String> {
    let out = Command::new("getprop")
        .arg(name)
        .output()
        .map_err(|e| format!("getprop {name}: {e}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Booted slot letter: "a" | "b" (from ro.boot.slot_suffix=_a/_b).
pub fn booted_slot() -> Result<char, String> {
    let s = getprop("ro.boot.slot_suffix")?;
    match s.as_str() {
        "_a" => Ok('a'),
        "_b" => Ok('b'),
        _ => Err(format!("ro.boot.slot_suffix unparsable: '{s}'")),
    }
}

/// Booted slot with override chain: explicit --booted-slot (TWRP passes
/// the bootctrl-HAL truth, unspoofable by resetprop/bind-mounts) ->
/// `bootctl get-current-slot` binary -> ro.boot.slot_suffix getprop.
/// The flag/first two stay truthful after a live slot switch; getprop
/// does not (it shows the spoofed session slot).
pub fn booted_slot_resolved(forced: Option<char>) -> Result<char, String> {
    if let Some(b) = forced {
        return match b {
            'a' | 'b' => Ok(b),
            _ => Err(format!("--booted-slot unparsable: '{b}'")),
        };
    }
    if let Ok(out) = std::process::Command::new("/system/bin/bootctl")
        .args(["get-current-slot"])
        .output()
    {
        if out.status.success() {
            match String::from_utf8_lossy(&out.stdout).trim() {
                "0" => return Ok('a'),
                "1" => return Ok('b'),
                _ => {}
            }
        }
    }
    booted_slot()
}

pub fn target_slot(booted: char) -> char {
    if booted == 'a' {
        'b'
    } else {
        'a'
    }
}

/// Universal block path for a slot-suffixed physical partition.
pub fn by_name(base: &str, slot: char) -> String {
    format!("/dev/block/by-name/{base}_{slot}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slots() {
        assert_eq!(target_slot('a'), 'b');
        assert_eq!(target_slot('b'), 'a');
        assert_eq!(by_name("boot", 'a'), "/dev/block/by-name/boot_a");
    }

    #[test]
    fn booted_forced_wins() {
        assert_eq!(booted_slot_resolved(Some('a')), Ok('a'));
        assert_eq!(booted_slot_resolved(Some('b')), Ok('b'));
        assert!(booted_slot_resolved(Some('x')).is_err());
    }
}
