//! fox_avbctl: standalone (zero-dependency) analog of AOSP avbctl.
//!
//! Operates on a vbmeta image file (or block device node) directly:
//! reads the 256-byte AvbVBMetaImageHeader, flips the flags word at
//! offset 120 (BE u32): bit0 = HASHTREE_DISABLED (verity),
//! bit1 = VERIFICATION_DISABLED (verification).
//! If the file does not start with AVB0 magic, falls back to the
//! AVBf footer (last 64 bytes) to locate the header, mirroring
//! load_top_level_vbmeta_header() in libavb_user.

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::process::ExitCode;

const HEADER_SIZE: usize = 256;
const FLAGS_OFF: usize = 120;
const FLAG_HASHTREE_DISABLED: u32 = 1 << 0;
const FLAG_VERIFICATION_DISABLED: u32 = 1 << 1;
const FOOTER_SIZE: u64 = 64;

const EX_OK: u8 = 0;
const EX_USAGE: u8 = 64;
const EX_SOFTWARE: u8 = 70;

#[derive(Clone, Copy, PartialEq)]
enum Cmd {
    GetVerity,
    EnableVerity,
    DisableVerity,
    GetVerification,
    EnableVerification,
    DisableVerification,
}

fn usage(prog: &str) {
    eprintln!(
        "{prog} - command-line tool for AVB (standalone, file-backed).\n\
         \n\
         Usage:\n  \
         {prog} [--image PATH] COMMAND\n  \
         {prog} COMMAND [--image PATH]\n  \
         {prog} COMMAND PATH\n\
         \n\
         Commands:\n  \
         get-verity              - Print whether verity is enabled.\n  \
         disable-verity          - Disable verity (set HASHTREE_DISABLED).\n  \
         enable-verity           - Enable verity (clear HASHTREE_DISABLED).\n  \
         get-verification        - Print whether verification is enabled.\n  \
         disable-verification    - Disable verification (set VERIFICATION_DISABLED).\n  \
         enable-verification     - Enable verification (clear VERIFICATION_DISABLED).\n\
         \n\
         Options:\n  \
         --image PATH, -i PATH   - vbmeta image or block device (default: VB_META_IMAGE env or ./vbmeta.img).\n  \
         --force                 - Accepted for avbctl compat, no effect on files."
    );
}

fn parse_cmd(s: &str) -> Option<Cmd> {
    match s {
        "get-verity" => Some(Cmd::GetVerity),
        "enable-verity" => Some(Cmd::EnableVerity),
        "disable-verity" => Some(Cmd::DisableVerity),
        "get-verification" => Some(Cmd::GetVerification),
        "enable-verification" => Some(Cmd::EnableVerification),
        "disable-verification" => Some(Cmd::DisableVerification),
        _ => None,
    }
}

// Locate the header: offset 0 if AVB0 magic, else via AVBf footer.
// Returns (header_offset, header_bytes).
fn load_header(f: &mut std::fs::File) -> Result<(u64, [u8; HEADER_SIZE]), String> {
    let mut hdr = [0u8; HEADER_SIZE];
    f.seek(SeekFrom::Start(0))
        .map_err(|e| format!("seek 0: {e}"))?;
    f.read_exact(&mut hdr)
        .map_err(|e| format!("read header: {e}"))?;
    if &hdr[0..4] == b"AVB0" {
        return Ok((0, hdr));
    }
    // Footer fallback: last 64 bytes, magic AVBf, vbmeta_offset at +20 (BE u64).
    let len = f
        .seek(SeekFrom::End(0))
        .map_err(|e| format!("seek end: {e}"))?;
    if len < FOOTER_SIZE {
        return Err("not a vbmeta image (no AVB0 magic, too small for footer)".into());
    }
    let mut foot = [0u8; 64];
    f.seek(SeekFrom::End(-(FOOTER_SIZE as i64)))
        .map_err(|e| format!("seek footer: {e}"))?;
    f.read_exact(&mut foot)
        .map_err(|e| format!("read footer: {e}"))?;
    if &foot[0..4] != b"AVBf" {
        return Err("not a vbmeta image (no AVB0 magic, no AVBf footer)".into());
    }
    let off = u64::from_be_bytes(foot[20..28].try_into().unwrap());
    f.seek(SeekFrom::Start(off))
        .map_err(|e| format!("seek vbmeta @{off}: {e}"))?;
    f.read_exact(&mut hdr)
        .map_err(|e| format!("read vbmeta @{off}: {e}"))?;
    if &hdr[0..4] != b"AVB0" {
        return Err(format!("bad vbmeta magic at footer offset {off}"));
    }
    Ok((off, hdr))
}

fn store_header(f: &mut std::fs::File, off: u64, hdr: &[u8; HEADER_SIZE]) -> Result<(), String> {
    f.seek(SeekFrom::Start(off))
        .map_err(|e| format!("seek write @{off}: {e}"))?;
    f.write_all(hdr)
        .map_err(|e| format!("write header: {e}"))?;
    f.flush().map_err(|e| format!("flush: {e}"))?;
    Ok(())
}

fn flags_of(hdr: &[u8; HEADER_SIZE]) -> u32 {
    u32::from_be_bytes(hdr[FLAGS_OFF..FLAGS_OFF + 4].try_into().unwrap())
}

fn set_flags(hdr: &mut [u8; HEADER_SIZE], flags: u32) {
    hdr[FLAGS_OFF..FLAGS_OFF + 4].copy_from_slice(&flags.to_be_bytes());
}

fn main() -> ExitCode {
    let prog = std::env::args()
        .next()
        .unwrap_or_else(|| "fox_avbctl".into());
    let mut image: Option<String> = None;
    let mut cmd: Option<Cmd> = None;
    let mut positional: Vec<String> = Vec::new();

    let mut it = std::env::args().skip(1).peekable();
    while let Some(a) = it.next() {
        if a == "--image" || a == "-i" {
            match it.next() {
                Some(p) => image = Some(p),
                None => {
                    eprintln!("missing value for {a}");
                    usage(&prog);
                    return ExitCode::from(EX_USAGE);
                }
            }
        } else if let Some(v) = a.strip_prefix("--image=") {
            image = Some(v.to_string());
        } else if a == "--force" {
            // compat no-op: lock checks only make sense via AvbOps on-device.
        } else if a == "-h" || a == "--help" || a == "help" {
            usage(&prog);
            return ExitCode::from(EX_OK);
        } else if a.starts_with('-') {
            eprintln!("unknown option: {a}");
            usage(&prog);
            return ExitCode::from(EX_USAGE);
        } else if parse_cmd(&a).is_some() {
            if cmd.is_some() {
                eprintln!("only one COMMAND allowed");
                return ExitCode::from(EX_USAGE);
            }
            cmd = parse_cmd(&a);
        } else {
            positional.push(a);
        }
    }

    // Bare path positional: `cmd PATH` (or lone PATH with cmd missing -> usage).
    if image.is_none() && positional.len() == 1 {
        if cmd.is_some() {
            image = positional.pop();
        } else if parse_cmd(&positional[0]).is_none() {
            // Single positional that is not a command: treat as image, still need cmd.
            image = positional.pop();
        }
    } else if !positional.is_empty() {
        eprintln!("unexpected argument: {}", positional[0]);
        usage(&prog);
        return ExitCode::from(EX_USAGE);
    }

    let cmd = match cmd {
        Some(c) => c,
        None => {
            usage(&prog);
            return ExitCode::from(EX_USAGE);
        }
    };
    let path = image
        .or_else(|| std::env::var("VB_META_IMAGE").ok())
        .unwrap_or_else(|| "vbmeta.img".to_string());

    let mut f = match OpenOptions::new().read(true).write(true).open(&path) {
        Ok(f) => f,
        Err(e) => {
            // get-* can work on read-only files; retry read-only.
            match cmd {
                Cmd::GetVerity | Cmd::GetVerification => match std::fs::File::open(&path) {
                    Ok(mut rf) => {
                        let mut hdr = [0u8; HEADER_SIZE];
                        if let Err(e2) = rf
                            .seek(SeekFrom::Start(0))
                            .and_then(|_| rf.read_exact(&mut hdr).map(|_| ()))
                        {
                            eprintln!("Error reading '{path}': {e2}");
                            return ExitCode::from(EX_SOFTWARE);
                        }
                        if &hdr[0..4] != b"AVB0" {
                            eprintln!("Error: '{path}' does not look like a vbmeta header.");
                            return ExitCode::from(EX_SOFTWARE);
                        }
                        let flags = flags_of(&hdr);
                        match cmd {
                            Cmd::GetVerity => println!(
                                "verity is {}.",
                                if flags & FLAG_HASHTREE_DISABLED == 0 {
                                    "enabled"
                                } else {
                                    "disabled"
                                }
                            ),
                            _ => println!(
                                "verification is {}.",
                                if flags & FLAG_VERIFICATION_DISABLED == 0 {
                                    "enabled"
                                } else {
                                    "disabled"
                                }
                            ),
                        }
                        return ExitCode::from(EX_OK);
                    }
                    Err(_) => {
                        eprintln!("Error opening '{path}': {e}");
                        return ExitCode::from(EX_SOFTWARE);
                    }
                },
                _ => {
                    eprintln!("Error opening '{path}' for update: {e}");
                    return ExitCode::from(EX_SOFTWARE);
                }
            }
        }
    };

    let (off, mut hdr) = match load_header(&mut f) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::from(EX_SOFTWARE);
        }
    };
    let flags = flags_of(&hdr);

    match cmd {
        Cmd::GetVerity => {
            println!(
                "verity is {} ({}).",
                if flags & FLAG_HASHTREE_DISABLED == 0 {
                    "enabled"
                } else {
                    "disabled"
                },
                path
            );
            ExitCode::from(EX_OK)
        }
        Cmd::GetVerification => {
            println!(
                "verification is {} ({}).",
                if flags & FLAG_VERIFICATION_DISABLED == 0 {
                    "enabled"
                } else {
                    "disabled"
                },
                path
            );
            ExitCode::from(EX_OK)
        }
        Cmd::EnableVerity | Cmd::DisableVerity => {
            let want_enable = cmd == Cmd::EnableVerity;
            let is_enabled = flags & FLAG_HASHTREE_DISABLED == 0;
            if is_enabled == want_enable {
                println!(
                    "verity is already {}.",
                    if is_enabled { "enabled" } else { "disabled" }
                );
                return ExitCode::from(EX_OK);
            }
            let mut nf = flags & !FLAG_HASHTREE_DISABLED;
            if !want_enable {
                nf |= FLAG_HASHTREE_DISABLED;
            }
            set_flags(&mut hdr, nf);
            if let Err(e) = store_header(&mut f, off, &hdr) {
                eprintln!("Error writing to '{path}': {e}");
                return ExitCode::from(EX_SOFTWARE);
            }
            println!(
                "Successfully {} verity ({}).",
                if want_enable { "enabled" } else { "disabled" },
                path
            );
            ExitCode::from(EX_OK)
        }
        Cmd::EnableVerification | Cmd::DisableVerification => {
            let want_enable = cmd == Cmd::EnableVerification;
            let is_enabled = flags & FLAG_VERIFICATION_DISABLED == 0;
            if is_enabled == want_enable {
                println!(
                    "verification is already {}.",
                    if is_enabled { "enabled" } else { "disabled" }
                );
                return ExitCode::from(EX_OK);
            }
            let mut nf = flags & !FLAG_VERIFICATION_DISABLED;
            if !want_enable {
                nf |= FLAG_VERIFICATION_DISABLED;
            }
            set_flags(&mut hdr, nf);
            if let Err(e) = store_header(&mut f, off, &hdr) {
                eprintln!("Error writing to '{path}': {e}");
                return ExitCode::from(EX_SOFTWARE);
            }
            println!(
                "Successfully {} verification ({}).",
                if want_enable { "enabled" } else { "disabled" },
                path
            );
            ExitCode::from(EX_OK)
        }
    }
}
