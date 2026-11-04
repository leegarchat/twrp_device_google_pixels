//! Minimal zip reader (factory path): EOCD + central directory + entry streaming.
//!
//! Only what the engine needs: STORED (method 0) and DEFLATE (method 8)
//! entries, CRC32 check, no encryption, no zip64. A `base` offset lets
//! the same reader open a STORED nested zip as its own archive (all
//! file seeks are `base + offset`).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use flate2::bufread::DeflateDecoder;

pub const METHOD_STORED: u16 = 0;
pub const METHOD_DEFLATED: u16 = 8;

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub method: u16,
    pub comp_size: u64,
    pub size: u64,
    pub crc: u32,
    pub header_off: u64,
}

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// Locate End-Of-Central-Directory by scanning the last 64 KiB + 22 B
/// of the `[base, base+len)` window. Candidates are validated (sane
/// count, directory inside the window, PK\x01\x02 at its start) so a
/// coincidental EOCD magic in compressed data cannot mislead us.
/// Handles the Zip64 EOCD locator (offsets beyond 4 GiB).
fn read_eocd_at(file: &mut File, base: u64, len: u64) -> Result<(u64, u64, u64), String> {
    let tail = std::cmp::min(len, 65536 + 22);
    let mut buf = vec![0u8; tail as usize];
    file.seek(SeekFrom::Start(base + len - tail)).map_err(|e| e.to_string())?;
    file.read_exact(&mut buf).map_err(|e| e.to_string())?;
    let mut i = buf.len();
    while i >= 22 {
        i -= 1;
        if buf[i] == 0x50 && buf[i + 1] == 0x4b && buf[i + 2] == 0x05 && buf[i + 3] == 0x06 {
            let mut count = u16le(&buf, i + 10) as u64;
            let mut cd_off = u32le(&buf, i + 16) as u64;
            // Zip64 EOCD locator right before the EOCD?
            if (count == 0xFFFF || cd_off == 0xFFFFFFFF) && i >= 20 {
                let l = i - 20;
                if buf[l] == 0x50 && buf[l + 1] == 0x4b && buf[l + 2] == 0x06 && buf[l + 3] == 0x07 {
                    let e64off = u64le(&buf, l + 8);
                    if base + e64off + 56 <= base + len {
                        let save = file.stream_position().map_err(|e| e.to_string())?;
                        let mut e64 = [0u8; 56];
                        file.seek(SeekFrom::Start(base + e64off)).map_err(|e| e.to_string())?;
                        if file.read_exact(&mut e64).is_ok() && u32le(&e64, 0) == 0x06064b50 {
                            count = u64le(&e64, 32);
                            cd_off = u64le(&e64, 48);
                        }
                        file.seek(SeekFrom::Start(save)).map_err(|e| e.to_string())?;
                    }
                }
            }
            if count == 0 || count > 1_000_000 || cd_off + 46 > len {
                continue;
            }
            // Validate: first central-directory entry must start with PK\x01\x02.
            let mut sig = [0u8; 4];
            let save = file.stream_position().map_err(|e| e.to_string())?;
            file.seek(SeekFrom::Start(base + cd_off)).map_err(|e| e.to_string())?;
            let ok = file.read_exact(&mut sig).is_ok() && sig == [0x50, 0x4b, 0x01, 0x02];
            file.seek(SeekFrom::Start(save)).map_err(|e| e.to_string())?;
            if ok {
                let cd_size = len.saturating_sub(cd_off);
                return Ok((count, cd_off, cd_size));
            }
        }
        if i == 0 {
            break;
        }
    }
    Err("EOCD not found".to_string())
}

fn u64le(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes([
        b[o], b[o + 1], b[o + 2], b[o + 3], b[o + 4], b[o + 5], b[o + 6], b[o + 7],
    ])
}

pub struct Zip {
    file: File,
    base: u64,
    pub entries: Vec<Entry>,
}

impl Zip {
    pub fn open(path: &Path) -> Result<Zip, String> {
        let file = File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        Self::open_window(file, 0, len)
    }

    /// Open a STORED sub-range as its own archive (nested image-*.zip).
    pub fn open_nested(path: &Path, base: u64, len: u64) -> Result<Zip, String> {
        let file = File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
        Self::open_window(file, base, len)
    }

    fn open_window(mut file: File, base: u64, len: u64) -> Result<Zip, String> {
        let (count, cd_off, _cd_size) = read_eocd_at(&mut file, base, len)?;
        let mut entries = Vec::with_capacity(count as usize);
        file.seek(SeekFrom::Start(base + cd_off)).map_err(|e| e.to_string())?;
        for _ in 0..count {
            let mut h = [0u8; 46];
            file.read_exact(&mut h).map_err(|e| e.to_string())?;
            if u32le(&h, 0) != 0x02014b50 {
                return Err("bad central directory signature".to_string());
            }
            let method = u16le(&h, 10);
            let crc = u32le(&h, 16); // central header: sig4 ver2x2 flag2 meth2 time2 date2 crc4@16
            let mut comp_size = u32le(&h, 20) as u64;
            let mut size = u32le(&h, 24) as u64;
            let fn_len = u16le(&h, 28) as usize;
            let ef_len = u16le(&h, 30) as usize;
            let fc_len = u16le(&h, 32) as usize;
            let mut header_off = u32le(&h, 42) as u64;
            let mut name = vec![0u8; fn_len];
            file.read_exact(&mut name).map_err(|e| e.to_string())?;
            let mut extra = vec![0u8; ef_len];
            file.read_exact(&mut extra).map_err(|e| e.to_string())?;
            let mut fcomment = vec![0u8; fc_len];
            file.read_exact(&mut fcomment).map_err(|e| e.to_string())?;
            if comp_size == 0xFFFFFFFF || size == 0xFFFFFFFF || header_off == 0xFFFFFFFF {
                // Zip64 extra field (tag 0x0001): 64-bit values in order
                // size, comp_size, header_off, disk (only those present).
                let mut p = 0usize;
                let need = [
                    size == 0xFFFFFFFF,
                    comp_size == 0xFFFFFFFF,
                    header_off == 0xFFFFFFFF,
                ];
                let mut slots = [&mut size, &mut comp_size, &mut header_off];
                while p + 4 <= extra.len() {
                    let tag = u16le(&extra, p);
                    let sz = u16le(&extra, p + 2) as usize;
                    p += 4;
                    if p + sz > extra.len() {
                        break;
                    }
                    if tag == 0x0001 {
                        let mut q = 0usize;
                        for (k, slot) in need.iter().zip(slots.iter_mut()) {
                            if *k && q + 8 <= sz {
                                **slot = u64le(&extra, p + q);
                                q += 8;
                            }
                        }
                        break;
                    }
                    p += sz;
                }
                if size == 0xFFFFFFFF || comp_size == 0xFFFFFFFF || header_off == 0xFFFFFFFF {
                    return Err("unresolved zip64 entry".to_string());
                }
            }
            entries.push(Entry {
                name: String::from_utf8_lossy(&name).into_owned(),
                method,
                comp_size,
                size,
                crc,
                header_off,
            });
        }
        Ok(Zip { file, base, entries })
    }

    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// Byte offset where the entry's data starts (past local header),
    /// relative to the archive base.
    pub fn data_off(&mut self, e: &Entry) -> Result<u64, String> {
        let mut lh = [0u8; 30];
        self.file
            .seek(SeekFrom::Start(self.base + e.header_off))
            .map_err(|e| e.to_string())?;
        self.file.read_exact(&mut lh).map_err(|e| e.to_string())?;
        if u32le(&lh, 0) != 0x04034b50 {
            return Err(format!("bad local header for {}", e.name));
        }
        let fn_len = u16le(&lh, 26) as u64;
        let ef_len = u16le(&lh, 28) as u64;
        Ok(e.header_off + 30 + fn_len + ef_len)
    }

    /// First `n` decompressed bytes of an entry (no full inflate, no
    /// CRC — for header sniffing only).
    pub fn read_prefix(&mut self, e: &Entry, n: usize) -> Result<Vec<u8>, String> {
        if e.method != METHOD_STORED && e.method != METHOD_DEFLATED {
            return Err(format!("unsupported method {} for {}", e.method, e.name));
        }
        let start = self.data_off(e)?;
        self.file
            .seek(SeekFrom::Start(self.base + start))
            .map_err(|e| e.to_string())?;
        let mut out = Vec::with_capacity(std::cmp::min(n, 65536));
        let mut chunk = vec![0u8; 1 << 20];
        if e.method == METHOD_STORED {
            let mut left = std::cmp::min(n as u64, e.size) as usize;
            while left > 0 {
                let k = std::cmp::min(left, chunk.len());
                self.file.read_exact(&mut chunk[..k]).map_err(|e| e.to_string())?;
                out.extend_from_slice(&chunk[..k]);
                left -= k;
            }
            return Ok(out);
        }
        let lim = Limited { f: &mut self.file, left: e.comp_size, buf: vec![0u8; 1 << 20], pos: 0, cap: 0 };
        let mut dec = DeflateDecoder::new(std::io::BufReader::with_capacity(1 << 16, lim));
        while out.len() < n {
            let k = std::cmp::min(chunk.len(), n - out.len());
            match dec.read(&mut chunk[..k]) {
                Ok(0) => break,
                Ok(m) => out.extend_from_slice(&chunk[..m]),
                Err(x) => return Err(format!("inflate {}: {x}", e.name)),
            }
        }
        Ok(out)
    }

    /// Stream-decompress an entry into `w`, chunk by chunk. Returns the
    /// CRC32 of the decompressed bytes for the caller to verify.
    pub fn extract_to<W: std::io::Write>(&mut self, e: &Entry, w: &mut W) -> Result<u32, String> {
        if e.method != METHOD_STORED && e.method != METHOD_DEFLATED {
            return Err(format!("unsupported method {} for {}", e.method, e.name));
        }
        let start = self.data_off(e)?;
        self.file
            .seek(SeekFrom::Start(self.base + start))
            .map_err(|e| e.to_string())?;
        let mut hasher = Crc::new();
        if e.method == METHOD_STORED {
            let mut buf = vec![0u8; 1 << 20];
            let mut left_out = e.size;
            while left_out > 0 {
                let n = std::cmp::min(left_out, buf.len() as u64) as usize;
                self.file.read_exact(&mut buf[..n]).map_err(|e| e.to_string())?;
                w.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                hasher.update(&buf[..n]);
                left_out -= n as u64;
            }
        } else {
            // DEFLATE: raw stream (no zlib wrapper), pulled in chunks.
            let lim = Limited { f: &mut self.file, left: e.comp_size, buf: vec![0u8; 1 << 20], pos: 0, cap: 0 };
            let mut dec = DeflateDecoder::new(std::io::BufReader::with_capacity(1 << 16, lim));
            let mut chunk = vec![0u8; 1 << 20];
            loop {
                match dec.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        w.write_all(&chunk[..n]).map_err(|e| e.to_string())?;
                        hasher.update(&chunk[..n]);
                    }
                    Err(x) => return Err(format!("inflate {}: {x}", e.name)),
                }
            }
        }
        Ok(hasher.finalize())
    }
}

struct Limited<'a> {
    f: &'a mut File,
    left: u64,
    buf: Vec<u8>,
    pos: usize,
    cap: usize,
}

impl<'a> Read for Limited<'a> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pos == self.cap {
            if self.left == 0 {
                return Ok(0);
            }
            let n = std::cmp::min(self.left, self.buf.len() as u64) as usize;
            self.f
                .read_exact(&mut self.buf[..n])
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
            self.pos = 0;
            self.cap = n;
            self.left -= n as u64;
        }
        let n = std::cmp::min(out.len(), self.cap - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

impl<'a> std::io::BufRead for Limited<'a> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        if self.pos == self.cap {
            if self.left == 0 {
                return Ok(&[]);
            }
            let mut one = [0u8; 1];
            if self.read(&mut one)? == 0 {
                return Ok(&[]);
            }
            self.pos -= 1;
        }
        Ok(&self.buf[self.pos..self.cap])
    }
    fn consume(&mut self, n: usize) {
        self.pos = std::cmp::min(self.pos + n, self.cap);
    }
}

// Local CRC32 (IEEE) to avoid extra dependencies. Public so the
// factory path can CRC-check entry streams without re-reading.
pub struct Crc(u32);
impl Crc {
    const fn table() -> [u32; 256] {
        let mut t = [0u32; 256];
        let mut i = 0;
        while i < 256 {
            let mut c = i as u32;
            let mut j = 0;
            while j < 8 {
                c = if c & 1 != 0 { 0xEDB88320 ^ (c >> 1) } else { c >> 1 };
                j += 1;
            }
            t[i] = c;
            i += 1;
        }
        t
    }
    pub fn new() -> Crc {
        Crc(0xFFFFFFFF)
    }
    pub fn update(&mut self, b: &[u8]) {
        const T: [u32; 256] = Crc::table();
        let mut c = self.0;
        for &x in b {
            c = T[((c ^ x as u32) & 0xFF) as usize] ^ (c >> 8);
        }
        self.0 = c;
    }
    pub fn finalize(&self) -> u32 {
        self.0 ^ 0xFFFFFFFF
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_known() {
        let mut h = Crc::new();
        h.update(b"123456789");
        assert_eq!(h.finalize(), 0xCBF43926);
        // chunked must match
        let mut h2 = Crc::new();
        for c in b"123456789".chunks(2) {
            h2.update(c);
        }
        assert_eq!(h2.finalize(), 0xCBF43926);
    }
}
