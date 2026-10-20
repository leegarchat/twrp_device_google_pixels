//! DeltaArchiveManifest walk: partitions, ops, extents, verdict.
//! Field numbers per system/update_engine/update_metadata.proto:
//! manifest: block_size=3, partitions=13
//! PartitionUpdate: name=1, run_postinstall=2, postinstall_path=3,
//!   filesystem_type=4, old_partition_info=6, new_partition_info=7,
//!   operations=8
//! PartitionInfo: size=1, hash=2
//! InstallOperation: type=1, data_offset=2, data_length=3,
//!   dst_extents=6 (Extent: start=1, num=2)
//! Op types: 0 REPLACE, 1 REPLACE_BZ, 4 SOURCE_COPY, 5 SOURCE_BSDIFF,
//!   6 ZERO, 7 DISCARD, 8 REPLACE_XZ, 9 PUFFDIFF, 10 BROTLI_BSDIFF,
//!   11 ZUCCHINI, 12/13 LZDIFF*, 14 REPLACE_ZSTD.

use crate::pb;
use std::collections::BTreeMap;

pub const OP_REPLACE: u32 = 0;
pub const OP_REPLACE_BZ: u32 = 1;
pub const OP_ZERO: u32 = 6;
pub const OP_DISCARD: u32 = 7;
pub const OP_REPLACE_XZ: u32 = 8;
pub const OP_REPLACE_ZSTD: u32 = 14;

/// Full-OTA op allow-list (mirrors peridot ProbeHighSpeedPayload).
pub const FULL_ALLOW: &[u32] = &[
    OP_REPLACE,
    OP_REPLACE_BZ,
    OP_ZERO,
    OP_DISCARD,
    OP_REPLACE_XZ,
    OP_REPLACE_ZSTD,
];

/// Subset we can actually decode (bzip2-rs covers REPLACE_BZ).
pub const DECODE_ALLOW: &[u32] = &[OP_REPLACE, OP_REPLACE_BZ, OP_ZERO, OP_DISCARD, OP_REPLACE_XZ, OP_REPLACE_ZSTD];

pub fn op_name(t: u32) -> &'static str {
    match t {
        0 => "REPLACE",
        1 => "REPLACE_BZ",
        4 => "SOURCE_COPY",
        5 => "SOURCE_BSDIFF",
        6 => "ZERO",
        7 => "DISCARD",
        8 => "REPLACE_XZ",
        9 => "PUFFDIFF",
        10 => "BROTLI_BSDIFF",
        11 => "ZUCCHINI",
        12 => "LZDIFF_BSDIFF",
        13 => "LZDIFF_PUFFDIFF",
        14 => "REPLACE_ZSTD",
        _ => "UNKNOWN",
    }
}

#[derive(Debug, Clone)]
pub struct Op {
    pub typ: u32,
    pub off: u64,
    pub len: u64,
    pub dst: Vec<(u64, u64)>,
    /// data_sha256_hash of the stored blob (may be empty in old payloads).
    pub data_hash: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Part {
    pub name: String,
    pub ops: Vec<Op>,
    pub has_old_info: bool,
    pub new_size: u64,
    pub new_hash: Vec<u8>,
}

impl Part {
    /// Expanded (on-target) size from dst extents.
    pub fn out_size(&self, bs: u64) -> u64 {
        self.ops
            .iter()
            .flat_map(|o| o.dst.iter())
            .map(|(s, n)| (s + n) * bs)
            .max()
            .unwrap_or(0)
    }
    pub fn dl_size(&self) -> u64 {
        self.ops
            .iter()
            .filter(|o| o.typ != OP_ZERO && o.typ != OP_DISCARD)
            .map(|o| o.len)
            .sum()
    }
    pub fn hist(&self) -> BTreeMap<u32, usize> {
        let mut m = BTreeMap::new();
        for o in &self.ops {
            *m.entry(o.typ).or_insert(0) += 1;
        }
        m
    }
}

fn parse_extent(buf: &[u8]) -> Result<(u64, u64), String> {
    let mut s = 0u64;
    let mut n = 0u64;
    for (f, v) in pb::fields(buf)? {
        match f {
            1 => s = pb::num(&v)?,
            2 => n = pb::num(&v)?,
            _ => {}
        }
    }
    Ok((s, n))
}

fn parse_op(buf: &[u8]) -> Result<Op, String> {
    let mut o = Op {
        typ: 99,
        off: 0,
        len: 0,
        dst: Vec::new(),
        data_hash: Vec::new(),
    };
    for (f, v) in pb::fields(buf)? {
        match f {
            1 => o.typ = pb::num(&v)? as u32,
            2 => o.off = pb::num(&v)?,
            3 => o.len = pb::num(&v)?,
            6 => o.dst.push(parse_extent(pb::blob(buf, &v)?)?),
            8 => o.data_hash = pb::blob(buf, &v)?.to_vec(),
            _ => {}
        }
    }
    Ok(o)
}

pub fn parse(manifest: &[u8]) -> Result<Vec<Part>, String> {
    let mut parts = Vec::new();
    for (f, v) in pb::fields(manifest)? {
        if f != 13 {
            continue;
        }
        let blob = pb::blob(manifest, &v)?;
        let mut p = Part {
            name: String::new(),
            ops: Vec::new(),
            has_old_info: false,
            new_size: 0,
            new_hash: Vec::new(),
        };
        for (sf, sv) in pb::fields(blob)? {
            match sf {
                1 => {
                    p.name = String::from_utf8_lossy(pb::blob(blob, &sv)?).into_owned();
                }
                6 => p.has_old_info = true,
                7 => {
                    let nib = pb::blob(blob, &sv)?;
                    for (nf, nv) in pb::fields(nib)? {
                        match nf {
                            1 => p.new_size = pb::num(&nv)?,
                            2 => p.new_hash = pb::blob(nib, &nv)?.to_vec(),
                            _ => {}
                        }
                    }
                }
                8 => p.ops.push(parse_op(pb::blob(blob, &sv)?)?),
                _ => {}
            }
        }
        if p.name.is_empty() {
            return Err("partition without name".to_string());
        }
        parts.push(p);
    }
    if parts.is_empty() {
        return Err("no partitions in manifest".to_string());
    }
    Ok(parts)
}

/// Dynamic partitions metadata (manifest field 15).
#[derive(Debug, Clone)]
pub struct DynGroup {
    pub name: String,
    pub size: u64,
    pub parts: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DynMeta {
    pub groups: Vec<DynGroup>,
    pub snapshot: bool,
    pub vabc: bool,
}

/// Parse manifest.dynamic_partition_metadata (field 15).
/// None = absent (non-dynamic payload).
pub fn parse_dynmeta(manifest: &[u8]) -> Result<Option<DynMeta>, String> {
    for (f, v) in pb::fields(manifest)? {
        if f != 15 {
            continue;
        }
        let blob = pb::blob(manifest, &v)?;
        let mut dm = DynMeta {
            groups: Vec::new(),
            snapshot: false,
            vabc: false,
        };
        for (df, dv) in pb::fields(blob)? {
            match df {
                1 => {
                    let gb = pb::blob(blob, &dv)?;
                    let mut g = DynGroup {
                        name: String::new(),
                        size: 0,
                        parts: Vec::new(),
                    };
                    for (gf, gv) in pb::fields(gb)? {
                        match gf {
                            1 => {
                                g.name =
                                    String::from_utf8_lossy(pb::blob(gb, &gv)?).into_owned();
                            }
                            2 => g.size = pb::num(&gv)?,
                            3 => g
                                .parts
                                .push(String::from_utf8_lossy(pb::blob(gb, &gv)?).into_owned()),
                            _ => {}
                        }
                    }
                    dm.groups.push(g);
                }
                2 => dm.snapshot = pb::num(&dv)? != 0,
                3 => dm.vabc = pb::num(&dv)? != 0,
                _ => {}
            }
        }
        return Ok(Some(dm));
    }
    Ok(None)
}
/// Ok(()) = full (our fast path eligible), Err(reason) = incremental-ish
/// (must take the stock update_engine path).
pub fn verdict_full(parts: &[Part]) -> Result<(), String> {
    for p in parts {
        if p.has_old_info {
            return Err(format!("{}: old_partition_info present (delta)", p.name));
        }
        for o in &p.ops {
            if !FULL_ALLOW.contains(&o.typ) {
                return Err(format!(
                    "{}: op {} ({}) not in full-OTA allow-list",
                    p.name,
                    o.typ,
                    op_name(o.typ)
                ));
            }
        }
    }
    Ok(())
}

/// DeltaArchiveManifest.minor_version (field 12, default 0).
/// Non-zero means a delta payload even if every op looks plain
/// (peridot parity) -> stock path, never a hard error.
pub fn minor_version(manifest: &[u8]) -> Result<u32, String> {
    for (f, v) in pb::fields(manifest)? {
        if f == 12 {
            return Ok(pb::num(&v)? as u32);
        }
    }
    Ok(0)
}

/// Exact-tiling check (peridot validate() parity): dst extents of every
/// partition must tile [0, new_size) with no gaps and no overlaps.
/// A stock update_engine would refuse these too, so a violation is a
/// hard error, never a stock fallback.
pub fn validate_tiling(parts: &[Part], bs: u64) -> Result<(), String> {
    for p in parts {
        if p.new_size == 0 {
            continue;
        }
        let mut total = 0u64;
        for o in &p.ops {
            for e in &o.dst {
                total = total.saturating_add(e.1);
            }
        }
        if total.saturating_mul(bs) != p.new_size {
            return Err(format!(
                "tiling: {}: extents cover {} bytes, new_size={}",
                p.name,
                total.saturating_mul(bs),
                p.new_size
            ));
        }
        let mut segs: Vec<(u64, u64)> = Vec::new();
        for o in &p.ops {
            for e in &o.dst {
                segs.push((e.0, e.0 + e.1));
            }
        }
        segs.sort();
        let mut expect = 0u64;
        for s in &segs {
            if s.0 != expect {
                return Err(format!(
                    "tiling: {}: gap/overlap at block {expect}",
                    p.name
                ));
            }
            expect = s.1;
        }
    }
    Ok(())
}
