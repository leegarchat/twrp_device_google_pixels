//! Local-zip location of payload.bin: EOCD -> central directory ->
//! local header -> payload header (CrAU). Mirrors payload_range_fetch.py
//! find_payload_offset, but with pread on a local file instead of HTTP.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

pub struct ZipLayout {
    pub file_len: u64,
    /// zip offset where payload.bin bytes start
    pub payload_off: u64,
    pub manifest_len: u64,
    pub sig_len: u64,
    pub manifest: Vec<u8>,
    pub block_size: u64,
    /// zip offset where op data blobs start
    pub data_start: u64,
}

fn read_at(f: &mut File, off: u64, len: usize) -> Result<Vec<u8>, String> {
    f.seek(SeekFrom::Start(off))
        .map_err(|e| format!("seek {off}: {e}"))?;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf)
        .map_err(|e| format!("read {off}+{len}: {e}"))?;
    Ok(buf)
}

/// Read a stored (uncompressed) zip entry by exact name.
/// Used for META-INF/com/android/metadata (pre-device check).
/// Compressed entries are refused loudly — metadata must be stored.
pub fn read_entry(path: &str, want: &str) -> Result<Vec<u8>, String> {
    let mut f = File::open(path).map_err(|e| format!("open {path}: {e}"))?;
    let total = f
        .metadata()
        .map_err(|e| format!("stat: {e}"))?
        .len();
    let tail_len = total.min(1 << 20);
    let tail = read_at(&mut f, total - tail_len, tail_len as usize)?;
    let base = total - tail_len;
    let mut eocd_rel: Option<usize> = None;
    for i in (0..tail.len().saturating_sub(21)).rev() {
        if tail[i] == 0x50 && tail[i + 1] == 0x4b && tail[i + 2] == 0x05 && tail[i + 3] == 0x06
        {
            eocd_rel = Some(i);
            break;
        }
    }
    let eocd_rel = eocd_rel.ok_or("EOCD not found (not a zip?)")?;
    let eocd = &tail[eocd_rel..];
    let cd_off = u32le(eocd, 16) as u64;
    let eocd_file_off = base + eocd_rel as u64;
    let cd = read_at(&mut f, cd_off, (eocd_file_off - cd_off) as usize)?;
    // Full central-directory walk (mirrors probe()).
    let mut pos = 0usize;
    while pos + 46 <= cd.len() {
        if &cd[pos..pos + 4] != b"PK\x01\x02" {
            break;
        }
        let method = u16le(&cd, pos + 10);
        let csize = u32le(&cd, pos + 24) as u64;
        let fnlen = u16le(&cd, pos + 28) as usize;
        let eflen = u16le(&cd, pos + 30) as usize;
        let fclen = u16le(&cd, pos + 32) as usize;
        let lho = u32le(&cd, pos + 42) as u64;
        let name = String::from_utf8_lossy(&cd[pos + 46..pos + 46 + fnlen]).into_owned();
        if name == want || name.trim_start_matches('/') == want {
            if method != 0 {
                return Err(format!("entry {want}: method {method} (only stored supported)"));
            }
            let lh = read_at(&mut f, lho, 30)?;
            if &lh[..4] != b"PK\x03\x04" {
                return Err(format!("entry {want}: bad local header"));
            }
            let lfn = u16le(&lh, 26) as u64;
            let lex = u16le(&lh, 28) as u64;
            return read_at(&mut f, lho + 30 + lfn + lex, csize as usize);
        }
        pos += 46 + fnlen + eflen + fclen;
    }
    Err(format!("entry {want} not found"))
}

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

pub fn probe(path: &str) -> Result<ZipLayout, String> {
    let mut f = File::open(path).map_err(|e| format!("open {path}: {e}"))?;
    let total = f
        .metadata()
        .map_err(|e| format!("stat: {e}"))?
        .len();

    // --- EOCD in last 1 MiB ---
    let tail_len = total.min(1 << 20);
    let tail = read_at(&mut f, total - tail_len, tail_len as usize)?;
    let base = total - tail_len;
    let mut eocd_rel: Option<usize> = None;
    // rfind PK\x05\x06 (inclusive of the empty-comment tail position)
    for i in (0..tail.len().saturating_sub(21)).rev() {
        if tail[i] == 0x50
            && tail[i + 1] == 0x4b
            && tail[i + 2] == 0x05
            && tail[i + 3] == 0x06
        {
            eocd_rel = Some(i);
            break;
        }
    }
    let eocd_rel = eocd_rel.ok_or("EOCD not found in last 1MB (not a zip?)")?;
    let eocd = &tail[eocd_rel..];
    let cd_off = u32le(eocd, 16) as u64;
    let eocd_file_off = base + eocd_rel as u64;

    // --- central directory walk ---
    let cd = read_at(&mut f, cd_off, (eocd_file_off - cd_off) as usize)?;
    let mut pos = 0usize;
    let mut payload_lho: Option<u64> = None;
    while pos + 46 <= cd.len() {
        if &cd[pos..pos + 4] != b"PK\x01\x02" {
            break;
        }
        let fnlen = u16le(&cd, pos + 28) as usize;
        let eflen = u16le(&cd, pos + 30) as usize;
        let fclen = u16le(&cd, pos + 32) as usize;
        let lho = u32le(&cd, pos + 42) as u64;
        let name =
            String::from_utf8_lossy(&cd[pos + 46..pos + 46 + fnlen]).into_owned();
        if name == "payload.bin" || name.ends_with("/payload.bin") {
            payload_lho = Some(lho);
            break;
        }
        pos += 46 + fnlen + eflen + fclen;
    }
    let lho = payload_lho.ok_or("payload.bin not in central directory")?;

    // --- local header: data starts after name+extra ---
    let lh = read_at(&mut f, lho, 30)?;
    if &lh[..4] != b"PK\x03\x04" {
        return Err("payload.bin local header magic mismatch".to_string());
    }
    let lfn = u16le(&lh, 26) as u64;
    let lex = u16le(&lh, 28) as u64;
    let p_off = lho + 30 + lfn + lex;

    // --- payload header CrAU ---
    let hdr = read_at(&mut f, p_off, 24)?;
    if &hdr[..4] != b"CrAU" {
        return Err("payload magic mismatch (not CrAU)".to_string());
    }
    let msize = u64::from_be_bytes(hdr[12..20].try_into().unwrap());
    let ssize = u32::from_be_bytes(hdr[20..24].try_into().unwrap()) as u64;
    let manifest = read_at(&mut f, p_off + 24, msize as usize)?;
    if manifest.len() as u64 != msize {
        return Err("short manifest read".to_string());
    }

    // block_size lives in the manifest (field 3); parse lazily here.
    let mut block_size = 4096u64;
    for (fld, v) in crate::pb::fields(&manifest)? {
        if fld == 3 {
            if let Ok(n) = crate::pb::num(&v) {
                block_size = n;
            }
        }
    }

    Ok(ZipLayout {
        file_len: total,
        payload_off: p_off,
        manifest_len: msize,
        sig_len: ssize,
        manifest,
        block_size,
        data_start: p_off + 24 + msize + ssize,
    })
}
