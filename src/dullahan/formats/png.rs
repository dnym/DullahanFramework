//! Zero-dependency PNG decoder (8-bit RGB/RGBA/Gray/Indexed + 16-bit Grayscale heightmaps)
//! and RGBA8 PNG encoder. Pure Rust RFC 1950/1951 inflate + PNG filter reconstruction.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

#[derive(Clone, Debug)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    /// Top-to-bottom RGBA8 pixels (`width * height * 4` bytes).
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct DecodedHeight16 {
    pub width: u32,
    pub height: u32,
    /// Top-to-bottom 16-bit elevation samples (`width * height`).
    pub samples: Vec<u16>,
}

pub fn load_png_rgba<P: AsRef<Path>>(path: P) -> Result<DecodedImage, String> {
    let bytes = fs::read(path.as_ref()).map_err(|e| format!("{}: {e}", path.as_ref().display()))?;
    decode_png_rgba(&bytes)
}

pub fn load_png_height16<P: AsRef<Path>>(path: P) -> Result<DecodedHeight16, String> {
    let bytes = fs::read(path.as_ref()).map_err(|e| format!("{}: {e}", path.as_ref().display()))?;
    decode_png_height16(&bytes)
}

struct ParsedPng {
    width: u32,
    height: u32,
    bit_depth: u8,
    color_type: u8,
    palette: Vec<[u8; 3]>,
    trns: Vec<u8>,
    scanlines: Vec<u8>,
}

fn parse_png_raw(bytes: &[u8]) -> Result<ParsedPng, String> {
    const SIG: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
    if bytes.len() < 33 || bytes[0..8] != SIG {
        return Err("invalid PNG signature".into());
    }

    let mut pos = 8usize;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut bit_depth = 0u8;
    let mut color_type = 0u8;
    let mut interlace = 0u8;
    let mut palette = Vec::new();
    let mut trns = Vec::new();
    let mut idat = Vec::new();
    let mut seen_ihdr = false;

    while pos + 12 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        let ctype = &bytes[pos + 4..pos + 8];
        pos += 8;
        if pos + len + 4 > bytes.len() {
            return Err("truncated PNG chunk".into());
        }
        let data = &bytes[pos..pos + len];
        pos += len + 4; // skip CRC

        match ctype {
            b"IHDR" => {
                if len != 13 {
                    return Err("invalid IHDR length".into());
                }
                width = u32::from_be_bytes(data[0..4].try_into().unwrap());
                height = u32::from_be_bytes(data[4..8].try_into().unwrap());
                bit_depth = data[8];
                color_type = data[9];
                if data[10] != 0 || data[11] != 0 {
                    return Err("unsupported PNG compression/filter method".into());
                }
                interlace = data[12];
                if interlace != 0 {
                    return Err("interlaced PNG (Adam7) not yet supported".into());
                }
                seen_ihdr = true;
            }
            b"PLTE" => {
                for chunk in data.chunks_exact(3) {
                    palette.push([chunk[0], chunk[1], chunk[2]]);
                }
            }
            b"tRNS" => {
                trns.extend_from_slice(data);
            }
            b"IDAT" => {
                idat.extend_from_slice(data);
            }
            b"IEND" => break,
            _ => {}
        }
    }

    if !seen_ihdr || width == 0 || height == 0 {
        return Err("missing or empty IHDR".into());
    }
    let _ = interlace;

    let channels: usize = match color_type {
        0 => 1, // Grayscale
        2 => 3, // RGB
        3 => 1, // Indexed
        4 => 2, // Gray + Alpha
        6 => 4, // RGBA
        _ => return Err(format!("unsupported PNG color_type={color_type}")),
    };
    if !matches!(bit_depth, 1 | 2 | 4 | 8 | 16) {
        return Err(format!("unsupported PNG bit_depth={bit_depth}"));
    }

    let bits_per_pixel = channels * bit_depth as usize;
    let row_bytes = (width as usize * bits_per_pixel).div_ceil(8);
    let bpp = bits_per_pixel.div_ceil(8); // filter byte stride

    let raw = zlib_inflate(&idat)?;
    let expected = height as usize * (row_bytes + 1);
    if raw.len() < expected {
        return Err(format!("PNG IDAT decompressed to {} bytes, expected {}", raw.len(), expected));
    }

    let mut scanlines = vec![0u8; height as usize * row_bytes];
    for y in 0..height as usize {
        let src_row = &raw[y * (row_bytes + 1)..(y + 1) * (row_bytes + 1)];
        let filter = src_row[0];
        let filt = &src_row[1..];
        let row_start = y * row_bytes;
        for x in 0..row_bytes {
            let a = if x >= bpp { scanlines[row_start + x - bpp] } else { 0 };
            let b = if y > 0 { scanlines[row_start - row_bytes + x] } else { 0 };
            let c = if x >= bpp && y > 0 { scanlines[row_start - row_bytes + x - bpp] } else { 0 };
            let recon = match filter {
                0 => filt[x],
                1 => filt[x].wrapping_add(a),
                2 => filt[x].wrapping_add(b),
                3 => filt[x].wrapping_add(((a as u16 + b as u16) >> 1) as u8),
                4 => filt[x].wrapping_add(paeth(a, b, c)),
                _ => return Err(format!("invalid PNG filter byte {filter} at row {y}")),
            };
            scanlines[row_start + x] = recon;
        }
    }

    Ok(ParsedPng {
        width,
        height,
        bit_depth,
        color_type,
        palette,
        trns,
        scanlines,
    })
}

pub fn decode_png_rgba(bytes: &[u8]) -> Result<DecodedImage, String> {
    let p = parse_png_raw(bytes)?;
    let w = p.width as usize;
    let h = p.height as usize;
    let mut rgba = vec![0u8; w * h * 4];
    let channels: usize = match p.color_type {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => unreachable!(),
    };
    let row_bytes = (w * channels * p.bit_depth as usize).div_ceil(8);

    for y in 0..h {
        let row = &p.scanlines[y * row_bytes..(y + 1) * row_bytes];
        for x in 0..w {
            let dst = (y * w + x) * 4;
            match (p.color_type, p.bit_depth) {
                (6, 8) => {
                    let s = x * 4;
                    rgba[dst..dst + 4].copy_from_slice(&row[s..s + 4]);
                }
                (6, 16) => {
                    let s = x * 8;
                    rgba[dst] = row[s];
                    rgba[dst + 1] = row[s + 2];
                    rgba[dst + 2] = row[s + 4];
                    rgba[dst + 3] = row[s + 6];
                }
                (2, 8) => {
                    let s = x * 3;
                    rgba[dst] = row[s];
                    rgba[dst + 1] = row[s + 1];
                    rgba[dst + 2] = row[s + 2];
                    rgba[dst + 3] = 255;
                }
                (2, 16) => {
                    let s = x * 6;
                    rgba[dst] = row[s];
                    rgba[dst + 1] = row[s + 2];
                    rgba[dst + 2] = row[s + 4];
                    rgba[dst + 3] = 255;
                }
                (4, 8) => {
                    let s = x * 2;
                    let g = row[s];
                    rgba[dst] = g;
                    rgba[dst + 1] = g;
                    rgba[dst + 2] = g;
                    rgba[dst + 3] = row[s + 1];
                }
                (0, 8) => {
                    let g = row[x];
                    rgba[dst] = g;
                    rgba[dst + 1] = g;
                    rgba[dst + 2] = g;
                    rgba[dst + 3] = 255;
                }
                (0, 16) => {
                    let g = row[x * 2];
                    rgba[dst] = g;
                    rgba[dst + 1] = g;
                    rgba[dst + 2] = g;
                    rgba[dst + 3] = 255;
                }
                (3, bd @ (1 | 2 | 4 | 8)) => {
                    let idx = read_subbyte(row, x, bd) as usize;
                    let rgb = p.palette.get(idx).copied().unwrap_or([255, 0, 255]);
                    let a = p.trns.get(idx).copied().unwrap_or(255);
                    rgba[dst] = rgb[0];
                    rgba[dst + 1] = rgb[1];
                    rgba[dst + 2] = rgb[2];
                    rgba[dst + 3] = a;
                }
                (0, bd @ (1 | 2 | 4)) => {
                    let max_v = ((1u16 << bd) - 1) as u16;
                    let v = (read_subbyte(row, x, bd) as u16 * 255 / max_v) as u8;
                    rgba[dst] = v;
                    rgba[dst + 1] = v;
                    rgba[dst + 2] = v;
                    rgba[dst + 3] = 255;
                }
                _ => {
                    return Err(format!(
                        "unsupported PNG color_type={} bit_depth={}",
                        p.color_type, p.bit_depth
                    ));
                }
            }
        }
    }

    Ok(DecodedImage { width: p.width, height: p.height, rgba })
}

pub fn decode_png_height16(bytes: &[u8]) -> Result<DecodedHeight16, String> {
    let p = parse_png_raw(bytes)?;
    let w = p.width as usize;
    let h = p.height as usize;
    let mut samples = vec![0u16; w * h];
    match (p.color_type, p.bit_depth) {
        (0, 16) => {
            for i in 0..w * h {
                samples[i] = u16::from_be_bytes([p.scanlines[i * 2], p.scanlines[i * 2 + 1]]);
            }
        }
        (0, 8) => {
            for i in 0..w * h {
                let v = p.scanlines[i] as u16;
                samples[i] = (v << 8) | v;
            }
        }
        _ => {
            // Fall back to luminance from RGBA decode
            let img = decode_png_rgba(bytes)?;
            for i in 0..w * h {
                let r = img.rgba[i * 4] as u16;
                samples[i] = (r << 8) | r;
            }
        }
    }
    Ok(DecodedHeight16 { width: p.width, height: p.height, samples })
}

#[inline]
fn read_subbyte(row: &[u8], x: usize, bit_depth: u8) -> u8 {
    let bd = bit_depth as usize;
    let bit_idx = x * bd;
    let byte = row[bit_idx / 8];
    let shift = 8 - bd - (bit_idx % 8);
    (byte >> shift) & ((1u8 << bd) - 1)
}

#[inline]
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let a = a as i32;
    let b = b as i32;
    let c = c as i32;
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

// ---------------------------------------------------------------------------
// Pure-Rust RFC 1950 (zlib) + RFC 1951 (DEFLATE) Inflate
// ---------------------------------------------------------------------------

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit_buf: u64,
    bit_cnt: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0, bit_buf: 0, bit_cnt: 0 }
    }

    fn refill(&mut self, n: u32) -> Result<(), String> {
        while self.bit_cnt < n {
            if self.pos >= self.data.len() {
                return Err("unexpected EOF in DEFLATE stream".into());
            }
            self.bit_buf |= (self.data[self.pos] as u64) << self.bit_cnt;
            self.pos += 1;
            self.bit_cnt += 8;
        }
        Ok(())
    }

    fn read_bits(&mut self, n: u32) -> Result<u32, String> {
        if n == 0 {
            return Ok(0);
        }
        self.refill(n)?;
        let val = (self.bit_buf & ((1u64 << n) - 1)) as u32;
        self.bit_buf >>= n;
        self.bit_cnt -= n;
        Ok(val)
    }

    fn align_byte(&mut self) {
        let drop_bits = self.bit_cnt & 7;
        self.bit_buf >>= drop_bits;
        self.bit_cnt -= drop_bits;
    }

    fn read_aligned_u16(&mut self) -> Result<u16, String> {
        let lo = self.read_bits(8)?;
        let hi = self.read_bits(8)?;
        Ok(((hi << 8) | lo) as u16)
    }

    fn read_aligned_bytes(&mut self, len: usize, out: &mut Vec<u8>) -> Result<(), String> {
        let mut remaining = len;
        while remaining > 0 && self.bit_cnt >= 8 {
            out.push((self.bit_buf & 0xFF) as u8);
            self.bit_buf >>= 8;
            self.bit_cnt -= 8;
            remaining -= 1;
        }
        if self.pos + remaining > self.data.len() {
            return Err("unexpected EOF in stored DEFLATE block".into());
        }
        out.extend_from_slice(&self.data[self.pos..self.pos + remaining]);
        self.pos += remaining;
        Ok(())
    }
}

struct HuffTree {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl HuffTree {
    fn from_lengths(lengths: &[u8]) -> Result<Self, String> {
        let mut counts = [0u16; 16];
        for &l in lengths {
            if l as usize >= 16 {
                return Err("invalid Huffman code length".into());
            }
            if l > 0 {
                counts[l as usize] += 1;
            }
        }
        let mut offsets = [0usize; 16];
        let mut total = 0usize;
        for bits in 1..16 {
            offsets[bits] = total;
            total += counts[bits] as usize;
        }
        let mut symbols = vec![0u16; total];
        for (sym, &l) in lengths.iter().enumerate() {
            if l > 0 {
                let idx = offsets[l as usize];
                symbols[idx] = sym as u16;
                offsets[l as usize] += 1;
            }
        }
        Ok(Self { counts, symbols })
    }

    fn decode(&self, r: &mut BitReader<'_>) -> Result<u16, String> {
        let mut code = 0i32;
        let mut first = 0i32;
        let mut index = 0i32;
        for len in 1..=15 {
            code |= r.read_bits(1)? as i32;
            let count = self.counts[len] as i32;
            if code - count < first {
                return Ok(self.symbols[(index + (code - first)) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err("invalid Huffman code in DEFLATE stream".into())
    }
}

fn zlib_inflate(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 6 {
        return Err("zlib stream too short".into());
    }
    let cmf = data[0];
    let flg = data[1];
    if (cmf & 0x0F) != 8 {
        return Err("unsupported zlib compression method".into());
    }
    if (((cmf as u16) << 8) | flg as u16) % 31 != 0 {
        return Err("invalid zlib header checksum".into());
    }
    let start = if (flg & 0x20) != 0 { 6 } else { 2 };
    if start > data.len() {
        return Err("truncated zlib header".into());
    }

    let mut reader = BitReader::new(&data[start..]);
    let mut out = Vec::new();

    const LEN_BASE: [u16; 29] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99,
        115, 131, 163, 195, 227, 258,
    ];
    const LEN_EXTRA: [u32; 29] = [
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
    ];
    const DIST_BASE: [u16; 30] = [
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025,
        1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
    ];
    const DIST_EXTRA: [u32; 30] = [
        0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12,
        13, 13,
    ];
    const CL_ORDER: [usize; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];

    loop {
        let bfinal = reader.read_bits(1)?;
        let btype = reader.read_bits(2)?;
        match btype {
            0 => {
                reader.align_byte();
                let len = reader.read_aligned_u16()?;
                let nlen = reader.read_aligned_u16()?;
                if len != !nlen {
                    return Err("corrupt stored DEFLATE block length".into());
                }
                reader.read_aligned_bytes(len as usize, &mut out)?;
            }
            1 | 2 => {
                let (lit_tree, dist_tree) = if btype == 1 {
                    let mut lit_len = [0u8; 288];
                    lit_len[0..144].fill(8);
                    lit_len[144..256].fill(9);
                    lit_len[256..280].fill(7);
                    lit_len[280..288].fill(8);
                    let dist_len = [5u8; 32];
                    (HuffTree::from_lengths(&lit_len)?, HuffTree::from_lengths(&dist_len)?)
                } else {
                    let hlit = reader.read_bits(5)? as usize + 257;
                    let hdist = reader.read_bits(5)? as usize + 1;
                    let hclen = reader.read_bits(4)? as usize + 4;
                    let mut cl_lens = [0u8; 19];
                    for i in 0..hclen {
                        cl_lens[CL_ORDER[i]] = reader.read_bits(3)? as u8;
                    }
                    let cl_tree = HuffTree::from_lengths(&cl_lens)?;
                    let total = hlit + hdist;
                    let mut lens = Vec::with_capacity(total);
                    while lens.len() < total {
                        let sym = cl_tree.decode(&mut reader)?;
                        match sym {
                            0..=15 => lens.push(sym as u8),
                            16 => {
                                let prev = *lens.last().ok_or("DEFLATE repeat with no prev")?;
                                let rep = reader.read_bits(2)? as usize + 3;
                                lens.resize(lens.len() + rep, prev);
                            }
                            17 => {
                                let rep = reader.read_bits(3)? as usize + 3;
                                lens.resize(lens.len() + rep, 0);
                            }
                            18 => {
                                let rep = reader.read_bits(7)? as usize + 11;
                                lens.resize(lens.len() + rep, 0);
                            }
                            _ => return Err("bad code-length symbol".into()),
                        }
                    }
                    if lens.len() != total {
                        return Err("DEFLATE code length table overflow".into());
                    }
                    (
                        HuffTree::from_lengths(&lens[..hlit])?,
                        HuffTree::from_lengths(&lens[hlit..])?,
                    )
                };

                loop {
                    let sym = lit_tree.decode(&mut reader)?;
                    if sym < 256 {
                        out.push(sym as u8);
                    } else if sym == 256 {
                        break;
                    } else {
                        let li = (sym - 257) as usize;
                        if li >= LEN_BASE.len() {
                            return Err("invalid DEFLATE length symbol".into());
                        }
                        let length = LEN_BASE[li] as usize + reader.read_bits(LEN_EXTRA[li])? as usize;
                        let di = dist_tree.decode(&mut reader)? as usize;
                        if di >= DIST_BASE.len() {
                            return Err("invalid DEFLATE distance symbol".into());
                        }
                        let dist = DIST_BASE[di] as usize + reader.read_bits(DIST_EXTRA[di])? as usize;
                        if dist == 0 || dist > out.len() {
                            return Err("DEFLATE distance out of range".into());
                        }
                        for _ in 0..length {
                            let b = out[out.len() - dist];
                            out.push(b);
                        }
                    }
                }
            }
            _ => return Err("invalid DEFLATE block type 3".into()),
        }
        if bfinal != 0 {
            break;
        }
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// PNG Encoder (RGBA8)
// ---------------------------------------------------------------------------

/// Writes a top-to-bottom RGBA8 pixel buffer to a valid PNG file.
pub fn write_rgba_png<P: AsRef<Path>>(path: P, width: u32, height: u32, rgba: &[u8]) -> io::Result<()> {
    let bytes = encode_rgba_png(width, height, rgba)?;
    let mut file = File::create(path)?;
    file.write_all(&bytes)
}

pub fn encode_rgba_png(width: u32, height: u32, rgba: &[u8]) -> io::Result<Vec<u8>> {
    let expected = (width as usize) * (height as usize) * 4;
    if rgba.len() != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("RGBA buffer len {} != expected {} ({}x{}x4)", rgba.len(), expected, width, height),
        ));
    }

    let row_bytes = (width as usize) * 4;
    let raw_len = (height as usize) * (row_bytes + 1);
    let mut raw = Vec::with_capacity(raw_len);
    for y in 0..height as usize {
        raw.push(0u8);
        let start = y * row_bytes;
        raw.extend_from_slice(&rgba[start..start + row_bytes]);
    }

    let zlib = zlib_store(&raw);

    let mut out = Vec::with_capacity(64 + zlib.len());
    out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);

    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = 6;
    write_chunk(&mut out, b"IHDR", &ihdr);
    write_chunk(&mut out, b"IDAT", &zlib);
    write_chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

fn write_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);
    let mut crc = crc32_update(0xFFFF_FFFF, chunk_type);
    crc = crc32_update(crc, data) ^ 0xFFFF_FFFF;
    out.extend_from_slice(&crc.to_be_bytes());
}

fn zlib_store(data: &[u8]) -> Vec<u8> {
    let max_block = 65535usize;
    let num_blocks = data.len().div_ceil(max_block).max(1);
    let mut out = Vec::with_capacity(data.len() + num_blocks * 5 + 6);
    out.push(0x78);
    out.push(0x01);

    if data.is_empty() {
        out.extend_from_slice(&[0x01, 0x00, 0x00, 0xFF, 0xFF]);
    } else {
        let mut offset = 0;
        while offset < data.len() {
            let len = (data.len() - offset).min(max_block);
            let is_final = offset + len >= data.len();
            out.push(if is_final { 0x01 } else { 0x00 });
            let len16 = len as u16;
            out.extend_from_slice(&len16.to_le_bytes());
            out.extend_from_slice(&(!len16).to_le_bytes());
            out.extend_from_slice(&data[offset..offset + len]);
            offset += len;
        }
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = (a + byte as u32) % MOD;
        b = (b + a) % MOD;
    }
    (b << 16) | a
}

fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_rgba_png() {
        let pixels = [255u8, 10, 20, 255, 30, 200, 40, 128, 50, 60, 240, 255, 255, 255, 255, 0];
        let encoded = encode_rgba_png(2, 2, &pixels).unwrap();
        let decoded = decode_png_rgba(&encoded).unwrap();
        assert_eq!(decoded.width, 2);
        assert_eq!(decoded.height, 2);
        assert_eq!(decoded.rgba, pixels);
    }
}
