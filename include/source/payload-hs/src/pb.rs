//! Minimal protobuf walker: enough for update_metadata.proto.
//! Mirrors payload_range_fetch.py pb_fields/read_varint.

#[derive(Debug, Clone)]
pub enum Val {
    Var(u64),
    /// offset + len into the source buffer
    Len(usize, usize),
}

pub fn read_varint(buf: &[u8], mut pos: usize) -> Result<(u64, usize), String> {
    let mut shift = 0u32;
    let mut val = 0u64;
    loop {
        if pos >= buf.len() {
            return Err("varint: truncated".to_string());
        }
        let b = buf[pos];
        pos += 1;
        val |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift > 63 {
            return Err("varint: overflow".to_string());
        }
    }
    Ok((val, pos))
}

pub fn fields(buf: &[u8]) -> Result<Vec<(u32, Val)>, String> {
    let mut pos = 0usize;
    let mut out = Vec::new();
    while pos < buf.len() {
        let (key, p) = read_varint(buf, pos)?;
        pos = p;
        let f = (key >> 3) as u32;
        match key & 7 {
            0 => {
                let (v, p) = read_varint(buf, pos)?;
                pos = p;
                out.push((f, Val::Var(v)));
            }
            1 => {
                if pos + 8 > buf.len() {
                    return Err("fixed64: truncated".to_string());
                }
                pos += 8;
                out.push((f, Val::Var(0)));
            }
            2 => {
                let (ln, p) = read_varint(buf, pos)?;
                pos = p;
                let ln = ln as usize;
                if pos + ln > buf.len() {
                    return Err("len-delim: truncated".to_string());
                }
                out.push((f, Val::Len(pos, ln)));
                pos += ln;
            }
            5 => {
                if pos + 4 > buf.len() {
                    return Err("fixed32: truncated".to_string());
                }
                pos += 4;
                out.push((f, Val::Var(0)));
            }
            w => return Err(format!("unsupported wire type {w}")),
        }
    }
    Ok(out)
}

pub fn blob<'a>(buf: &'a [u8], v: &Val) -> Result<&'a [u8], String> {
    match v {
        Val::Len(o, l) => buf
            .get(*o..*o + *l)
            .ok_or_else(|| "blob: out of range".to_string()),
        _ => Err("blob: not a length-delimited field".to_string()),
    }
}

pub fn num(v: &Val) -> Result<u64, String> {
    match v {
        Val::Var(n) => Ok(*n),
        _ => Err("num: not a varint field".to_string()),
    }
}

/// First value of field `f`, if present.
pub fn first<'a>(fs: &'a [(u32, Val)], f: u32) -> Option<&'a Val> {
    fs.iter().find(|(n, _)| *n == f).map(|(_, v)| v)
}
