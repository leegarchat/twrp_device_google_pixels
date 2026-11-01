//! FBPK container parsing (factory path) (bootloader v2, radio v1).
//!
//! v1 (radio): header + single modem blob running to EOF.
//!   entry = name[32] + 5xu32 (52 B); blob = [entry_end : EOF].
//! v2 (bootloader): table1 = 104 B records (name[64] + 10xu32;
//!   file data_off = u32[3], data_size = u32[5]; blobs sequential,
//!   last one closes the file). table2 = 136 B slot records
//!   (names + raw tails = target geometry; blobs alias table1,
//!   except dram_train/devinfo/mfg_data which need special handling).

pub struct Blob {
    pub name: String,
    /// File offset of the blob payload.
    pub off: u64,
    /// File size of the blob payload.
    pub size: u64,
}

pub struct Table2Entry {
    pub name: String,
    /// Raw 10xu32 tail (target geometry, not file offsets).
    pub tail: [u32; 10],
}

pub struct Fbpk {
    pub version: u32,
    pub board: String,
    pub verstr: String,
    pub table1: Vec<Blob>,
    pub table2: Vec<Table2Entry>,
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn cstr(b: &[u8]) -> Option<String> {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    let s = &b[..end];
    if s.is_empty() || s.len() > 40 {
        return None;
    }
    if !s.iter().all(|&c| c.is_ascii_alphanumeric() || c == b'_' || c == b':' || c == b'-' || c == b'.') {
        return None;
    }
    Some(String::from_utf8_lossy(s).into_owned())
}

pub fn parse(data: &[u8], file_len: u64) -> Result<Fbpk, String> {
    if data.len() < 8 || &data[0..4] != b"FBPK" {
        return Err("not an FBPK container".to_string());
    }
    let ver = u32le(data, 4);
    if ver == 1 {
        let board = cstr(&data[8..72.min(data.len())]).unwrap_or_default();
        let at = data[..1024.min(data.len())]
            .windows(6)
            .position(|w| w == b"modem\x00")
            .ok_or("v1: no modem entry".to_string())?;
        let end = at as u64 + 32 + 20;
        Ok(Fbpk {
            version: 1,
            board,
            verstr: String::new(),
            table1: vec![Blob { name: "modem".to_string(), off: end, size: file_len - end }],
            table2: vec![],
        })
    } else if ver == 2 {
        let board = cstr(&data[16..32.min(data.len())]).unwrap_or_default();
        let verstr = cstr(&data[32..96.min(data.len())]).unwrap_or_default();
        // table1: 104 B records from offset 116, entry count from header.
        let t1count = u32le(data, 100) as usize;
        let mut table1 = Vec::new();
        let mut off = 116usize;
        for _ in 0..t1count.min(64) {
            if off + 104 > data.len() {
                break;
            }
            let nm = match cstr(&data[off..off + 64]) {
                Some(n) => n,
                None => break,
            };
            let mut tail = [0u32; 10];
            for (i, t) in tail.iter_mut().enumerate() {
                *t = u32le(data, off + 64 + i * 4);
            }
            // data_off = u32[3], data_size = u32[5] (verified file offsets).
            let (doff, dsize) = (tail[3] as u64, tail[5] as u64);
            if dsize == 0 || doff + dsize > file_len {
                // Insane range: stop, do not trust further records.
                break;
            }
            table1.push(Blob { name: nm, off: doff, size: dsize });
            off += 104;
        }
        // table2: first bl1_a past table1, 136 B stride, valid names only.
        let mut table2 = Vec::new();
        let mut pos = data[off.min(data.len())..]
            .windows(6)
            .position(|w| w == b"bl1_a\x00")
            .map(|p| off + p)
            .unwrap_or(usize::MAX);
        while pos + 136 <= data.len() && table2.len() < 64 {
            let nm = match cstr(&data[pos..pos + 32]) {
                Some(n) if !n.is_empty() && n.len() <= 24 && n.replace('_', "").chars().all(|c| c.is_alphanumeric()) => n,
                _ => break,
            };
            let mut tail = [0u32; 10];
            for (i, t) in tail.iter_mut().enumerate() {
                *t = u32le(data, pos + 96 + i * 4);
            }
            table2.push(Table2Entry { name: nm, tail });
            pos += 136;
        }
        Ok(Fbpk { version: 2, board, verstr, table1, table2 })
    } else {
        Err(format!("unknown FBPK version {ver}"))
    }
}
