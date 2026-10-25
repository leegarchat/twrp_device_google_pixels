//! Parallel partition fetch: pread blobs from the local zip, decode,
//! scatter-write at dst extents. Fresh sparse output => ZERO/DISCARD
//! need no writes at all. Mirrors otaripper direct-write semantics,
//! but targets files (host test); the block-device switch is one
//! open() call away in recovery.

use crate::manifest::{Op, Part, OP_DISCARD, OP_REPLACE, OP_REPLACE_BZ, OP_REPLACE_XZ, OP_REPLACE_ZSTD, OP_ZERO};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::time::Instant;

pub struct FetchStat {
    pub name: String,
    pub output_bytes: u64,
    pub downloaded_bytes: u64,
    pub millis: u128,
}

fn read_blob(zip_path: &str, off: u64, len: u64) -> Result<Vec<u8>, String> {
    let mut f = File::open(zip_path).map_err(|e| format!("open: {e}"))?;
    f.seek(SeekFrom::Start(off))
        .map_err(|e| format!("seek {off}: {e}"))?;
    let mut buf = vec![0u8; len as usize];
    f.read_exact(&mut buf)
        .map_err(|e| format!("read {off}+{len}: {e}"))?;
    Ok(buf)
}

fn decode(typ: u32, blob: Vec<u8>) -> Result<Vec<u8>, String> {
    match typ {
        OP_REPLACE => {
            // Exact size is verified by the caller against dst extents.
            Ok(blob)
        }
        OP_REPLACE_BZ => {
            // crabz2 (pure Rust, no C): bzip2-rs 0.1.2 chokes on some
            // real-world OTA streams ("huffman bitstream truncated" on
            // valid input), crabz2 decodes the full corpus.
            crabz2::decompress(&blob).map_err(|e| format!("bz decode: {e}"))
        }
        OP_REPLACE_XZ => {
            let mut r = lzma_rust2::XzReader::new(&blob[..], false);
            let mut out = Vec::new();
            r.read_to_end(&mut out)
                .map_err(|e| format!("xz decode: {e}"))?;
            Ok(out)
        }
        OP_REPLACE_ZSTD => {
            let mut r = ruzstd::decoding::StreamingDecoder::new(&blob[..])
                .map_err(|e| format!("zstd init: {e:?}"))?;
            let mut out = Vec::new();
            r.read_to_end(&mut out)
                .map_err(|e| format!("zstd decode: {e}"))?;
            Ok(out)
        }
        t => Err(format!("op {t}: unsupported op (delta-only OTA?)")),
    }
}

pub fn fetch_decode(
    zip_path: &str,
    data_start: u64,
    op: &Op,
) -> Result<Vec<u8>, String> {
    let blob = read_blob(zip_path, data_start + op.off, op.len)?;
    if !op.data_hash.is_empty() {
        if op.data_hash.len() != 32 {
            return Err("op data hash malformed".to_string());
        }
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(&blob);
        let sum = h.finalize();
        if sum[..] != op.data_hash[..] {
            return Err("op data sha256 mismatch".to_string());
        }
    }
    decode(op.typ, blob)
}

pub fn extract_partition(
    zip_path: &str,
    data_start: u64,
    bs: u64,
    part: &Part,
    out_path: &str,
) -> Result<FetchStat, String> {
    extract_partition_into(zip_path, data_start, bs, part, out_path, true, None)
}

/// Block-device variant: no truncate (nodes are fixed size), ZERO
/// extents are written explicitly, DISCARD stays a no-op.
/// `hasher` collects the expanded bytes for manifest-hash validation.
pub fn extract_partition_into(
    zip_path: &str,
    data_start: u64,
    bs: u64,
    part: &Part,
    out_path: &str,
    sparse: bool,
    hash: Option<&mut sha2::Sha256>,
) -> Result<FetchStat, String> {
    use sha2::Digest as _;
    let mut hash = hash;
    for o in &part.ops {
        if o.typ != OP_ZERO
            && o.typ != OP_DISCARD
            && !crate::manifest::DECODE_ALLOW.contains(&o.typ)
        {
            return Err(format!(
                "{}: op {} needs a decoder we did not vendor (munmap this part)",
                part.name, o.typ
            ));
        }
    }
    let t0 = Instant::now();
    let total_out = part.out_size(bs);
    if sparse {
        let out = File::create(out_path).map_err(|e| format!("create {out_path}: {e}"))?;
        out.set_len(total_out)
            .map_err(|e| format!("truncate: {e}"))?;
        drop(out);
    }

    let mut downloaded = 0u64;
    // Own handle per partition => independent cursor, no locking.
    // O_EXCL: on block devices this is an exclusive claim (fails EBUSY
    // if mounted/held open); on regular files without O_CREAT it is a
    // no-op, so the host sparse-file path is unaffected.
    let mut f = File::options()
        .write(true)
        .custom_flags(libc::O_EXCL)
        .open(out_path)
        .map_err(|e| {
            if e.raw_os_error() == Some(libc::EBUSY) {
                format!("reopen {out_path}: busy (mounted or held open)")
            } else {
                format!("reopen {out_path}: {e}")
            }
        })?;
    let zeros = vec![0u8; (bs as usize) * 32];
    for (i, o) in part.ops.iter().enumerate() {
        match o.typ {
            OP_ZERO if sparse => continue, // sparse file already reads zero
            OP_DISCARD => continue,
            OP_ZERO => {
                // block node: zero explicitly, honoring every extent.
                for (sb, nb) in &o.dst {
                    f.seek(SeekFrom::Start(sb * bs))
                        .map_err(|e| format!("oseek: {e}"))?;
                    let mut left = *nb * bs;
                    while left > 0 {
                        let n = (left as usize).min(zeros.len());
                        f.write_all(&zeros[..n])
                            .map_err(|e| format!("zero fill: {e}"))?;
                        if let Some(h) = hash.as_mut() {
                            h.update(&zeros[..n]);
                        }
                        left -= n as u64;
                    }
                }
                continue;
            }
            _ => {
                let blob = read_blob(zip_path, data_start + o.off, o.len)?;
                downloaded += o.len;
                // Per-op input hash (peridot parity): verify the stored
                // blob before decoding it.
                if !o.data_hash.is_empty() {
                    if o.data_hash.len() != 32 {
                        return Err(format!("op {i}: op data hash malformed"));
                    }
                    use sha2::Digest;
                    let mut h = sha2::Sha256::new();
                    h.update(&blob);
                    let sum = h.finalize();
                    if sum[..] != o.data_hash[..] {
                        return Err(format!("op {i}: op data sha256 mismatch"));
                    }
                }
                let plain = decode(o.typ, blob)?;
                let mut pos = 0usize;
                for (sb, nb) in &o.dst {
                    let n = (*nb * bs) as usize;
                    let chunk = plain
                        .get(pos..pos + n)
                        .ok_or_else(|| format!("op {i}: short decode (want {n})"))?;
                    f.seek(SeekFrom::Start(sb * bs))
                        .map_err(|e| format!("oseek: {e}"))?;
                    f.write_all(chunk).map_err(|e| format!("owrite: {e}"))?;
                    if let Some(h) = hash.as_mut() {
                        h.update(chunk);
                    }
                    pos += n;
                }
                if pos != plain.len() {
                    return Err(format!(
                        "op {i}: decode trailing {} bytes (manifest/extents mismatch?)",
                        plain.len() - pos
                    ));
                }
            }
        }
    }
    f.flush().map_err(|e| format!("flush: {e}"))?;
    Ok(FetchStat {
        name: part.name.clone(),
        output_bytes: total_out,
        downloaded_bytes: downloaded,
        millis: t0.elapsed().as_millis(),
    })
}

/// SHA-256 of the first `size` bytes of a node/file, to check against
/// new_partition_info.hash. Block nodes may be larger than the payload
/// image they carry, so the hash window must be explicit.
/// Also reports whether every byte read was zero (sanity: a real image
/// is never all zeros; hashing already touches every byte, so this is
/// a free piggyback with no extra I/O).
pub fn sha256_file_n(path: &str, size: u64) -> Result<([u8; 32], bool), String> {
    use sha2::Digest;
    let mut f = File::open(path).map_err(|e| format!("open: {e}"))?;
    let mut h = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut left = size;
    let mut all_zero = size > 0;
    while left > 0 {
        let n = (left as usize).min(buf.len());
        f.read_exact(&mut buf[..n])
            .map_err(|e| format!("read: {e}"))?;
        if all_zero && buf[..n].iter().any(|&b| b != 0) {
            all_zero = false;
        }
        h.update(&buf[..n]);
        left -= n as u64;
    }
    let sum = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&sum);
    Ok((out, all_zero))
}

/// SHA-256 of a finished image, to check against new_partition_info.hash.
pub fn sha256_file(path: &str) -> Result<[u8; 32], String> {
    use sha2::Digest;
    let mut f = File::open(path).map_err(|e| format!("open: {e}"))?;
    let mut h = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().into())
}
