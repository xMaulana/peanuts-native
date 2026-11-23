//! Zero-copy, multi-threaded shader container scanner and streaming package writer.
//!
//! Mirrors the binary formats, key hashing (`skey`), signature checking (`shader_sig`),
//! and `BTCDBTCD` package output of `dm/gfx/tools/shader_split.py` while keeping
//! memory usage minimal via `memmap2` and streaming I/O.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Mutex;

use flate2::read::DeflateDecoder;
use memmap2::Mmap;
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use std::io::Read;

pub const CTL_MAGIC: [u8; 4] = [0x34, 0x12, 0x76, 0x98];
pub const GFX_MAGIC: [u8; 4] = [0x78, 0x56, 0x34, 0x12];
pub const CS_MAGIC: [u8; 4] = [0x79, 0x56, 0x34, 0x12];
pub const NVN2_KEY_CTL: usize = 0x48;

const STAGES: [&str; 6] = ["vs", "hs", "ds", "gs", "ps", "cs"];
const NEST_LIMIT: usize = 16;

#[inline]
fn u16_le(d: &[u8], at: usize) -> Option<u16> {
    d.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}

#[inline]
fn u32_le(d: &[u8], at: usize) -> Option<u32> {
    d.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

#[inline]
fn i32_le(d: &[u8], at: usize) -> Option<i32> {
    d.get(at..at + 4)
        .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

#[inline]
fn u64_le(d: &[u8], at: usize) -> Option<u64> {
    d.get(at..at + 8)
        .map(|b| u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
}

#[inline]
fn u32_be(d: &[u8], at: usize) -> Option<u32> {
    d.get(at..at + 4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

#[inline]
pub fn nvn2_ctl(ctl: &[u8]) -> bool {
    ctl.len() >= NVN2_KEY_CTL && ctl[..4] == CTL_MAGIC && ctl[4..8] == [0xc9, 0, 0, 0]
}

#[inline]
pub fn ctl_len(ctl: &[u8]) -> usize {
    if nvn2_ctl(ctl) {
        return NVN2_KEY_CTL;
    }
    let n = u32_le(ctl, 0x20).unwrap_or(0) as usize;
    if (0x760..0x880).contains(&n) {
        n
    } else {
        0x880.min(ctl.len())
    }
}

pub fn key_ctl(ctl: &[u8], key_skip: (usize, usize)) -> Vec<u8> {
    let n = ctl_len(ctl).min(ctl.len());
    let mut c = ctl[..n].to_vec();
    if nvn2_ctl(ctl) {
        if c.len() >= 0x34 {
            c[0x30..0x34].fill(0);
        }
        return c;
    }
    let (skip_off, skip_len) = key_skip;
    if skip_len > 0 && skip_off + skip_len <= c.len() {
        c[skip_off..skip_off + skip_len].fill(0);
    }
    c
}

pub fn skey(ctl: &[u8], code: &[u8], key_skip: (usize, usize)) -> [u8; 8] {
    let kc = key_ctl(ctl, key_skip);
    let mut h = Sha256::new();
    h.update(&kc);
    h.update(code);
    let digest = h.finalize();
    let mut out = [0u8; 8];
    out.copy_from_slice(&digest[..8]);
    out
}

pub fn skey_hex(key: &[u8; 8]) -> String {
    let mut s = String::with_capacity(16);
    for &b in key {
        use std::fmt::Write;
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

pub fn shader_sig(ctl: &[u8], code: &[u8], key_skip: (usize, usize)) -> [u8; 32] {
    let c: Vec<u8> = if nvn2_ctl(ctl) {
        let mut v = Vec::with_capacity(1 + NVN2_KEY_CTL);
        v.push(b'N');
        v.extend_from_slice(&key_ctl(ctl, key_skip));
        v
    } else {
        let own = if ctl.len() >= 0x24 {
            u32_le(ctl, 0x20).unwrap_or(0) as usize
        } else {
            ctl.len()
        };
        let n = if (0x760..=ctl.len()).contains(&own) {
            own
        } else {
            ctl.len()
        };
        let (o, ln) = key_skip;
        let mut v = Vec::with_capacity(1 + n);
        v.push(b'C');
        if ln > 0 && o + ln <= n {
            v.extend_from_slice(&ctl[..o]);
            v.extend_from_slice(&ctl[o + ln..n]);
        } else {
            v.extend_from_slice(&ctl[..n]);
        }
        v
    };

    let mut h = Sha256::new();
    h.update((code.len() as u64).to_le_bytes());
    h.update((c.len() as u64).to_le_bytes());
    h.update(code);
    h.update(&c);
    let digest = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

pub fn write_package_record<W: Write>(w: &mut W, ctl: &[u8], code: &[u8]) -> std::io::Result<()> {
    let mut hdr = [0u8; 0x38];
    hdr[..8].copy_from_slice(b"BTCDBTCD");
    hdr[0x28..0x2c].copy_from_slice(&(ctl.len() as u32).to_le_bytes());
    hdr[0x2c..0x30].copy_from_slice(&(code.len() as u32).to_le_bytes());
    w.write_all(&hdr)?;
    w.write_all(code)?;
    w.write_all(ctl)?;
    Ok(())
}

pub fn sph_stage(code: &[u8]) -> Option<&'static str> {
    if code.len() < 0x34 {
        return None;
    }
    let w = u32_le(code, 0x30)?;
    let st = (w >> 10) & 0xf;
    if st == 0 && (w & 0x1f) == 1 {
        return Some("vs");
    }
    match st {
        1 => Some("vs"),
        2 => Some("hs"),
        3 => Some("ds"),
        4 => Some("gs"),
        5 => Some("ps"),
        _ => None,
    }
}

pub fn nvn2_stage(ctl: &[u8], code: &[u8]) -> Option<&'static str> {
    if ctl.len() > 0x56 {
        match ctl[0x56] {
            5 => return Some("cs"),
            6 => return Some("ts"),
            7 => return Some("ms"),
            _ => {}
        }
    }
    if code.len() >= 4 {
        let mut fake = [0u8; 0x34];
        fake[0x30..0x34].copy_from_slice(&code[..4]);
        return sph_stage(&fake);
    }
    None
}

pub fn check_shader(stage: &str, ctl: &[u8], code: &[u8]) -> Result<(), &'static str> {
    if ctl.len() < 0x24 || ctl[..4] != CTL_MAGIC || ctl.len() < ctl_len(ctl) {
        return Err("bad control section");
    }
    if nvn2_ctl(ctl) {
        let expected = u32_le(ctl, 0x44).unwrap_or(0) as usize;
        return if expected == code.len() {
            Ok(())
        } else {
            Err("control +0x44 does not match the code area length")
        };
    }
    let magic = code.get(..4);
    if (stage == "cs"
        && matches!(magic, Some(b) if b == CS_MAGIC || b == GFX_MAGIC)
        && magic != Some(&CS_MAGIC))
        || (stage != "cs" && magic != Some(&GFX_MAGIC))
    {
        return Err("code area header magic does not match the stage");
    }
    let expected = u32_le(ctl, 0x704).unwrap_or(0) as usize;
    if expected != code.len() {
        return Err("control +0x704 does not match the code area length");
    }
    Ok(())
}

pub struct RawShader<'a> {
    pub where_str: String,
    pub stage: &'static str,
    pub ctl: &'a [u8],
    pub code: &'a [u8],
}

pub struct OwnedShader {
    pub where_str: String,
    pub stage: &'static str,
    pub ctl: Vec<u8>,
    pub code: Vec<u8>,
}

pub fn yaz0_decompress(d: &[u8]) -> Result<Vec<u8>, &'static str> {
    if d.len() < 16 || &d[..4] != b"Yaz0" {
        return Err("not Yaz0");
    }
    let out_len = u32_be(d, 4).unwrap_or(0) as usize;
    let mut out = Vec::with_capacity(out_len);
    let mut src = 16usize;
    let mut mask = 0u8;
    let mut bits = 0u8;

    while out.len() < out_len && src < d.len() {
        if bits == 0 {
            mask = d[src];
            src += 1;
            bits = 8;
        }
        if (mask & 0x80) != 0 {
            if src >= d.len() {
                break;
            }
            out.push(d[src]);
            src += 1;
        } else {
            if src + 1 >= d.len() {
                break;
            }
            let b0 = d[src] as usize;
            let b1 = d[src + 1] as usize;
            src += 2;
            let dist = ((b0 & 0x0f) << 8) | b1;
            let mut count = b0 >> 4;
            if count == 0 {
                if src >= d.len() {
                    break;
                }
                count = d[src] as usize + 0x12;
                src += 1;
            } else {
                count += 2;
            }
            let mut copy_pos = out.len().checked_sub(dist + 1).ok_or("Yaz0 bad backreference")?;
            for _ in 0..count {
                if out.len() >= out_len {
                    break;
                }
                let b = out[copy_pos];
                out.push(b);
                copy_pos += 1;
            }
        }
        mask <<= 1;
        bits -= 1;
    }
    Ok(out)
}

pub fn sarc_members<'a>(d: &'a [u8]) -> Result<Vec<(String, &'a [u8])>, &'static str> {
    if d.len() < 20 || &d[..4] != b"SARC" {
        return Err("not SARC");
    }
    let hl = u16_le(d, 4).ok_or("SARC header")? as usize;
    let doff = u32_le(d, 12).ok_or("SARC doff")? as usize;
    let n = u16_le(d, hl + 6).ok_or("SFAT header")? as usize;
    let names = hl + 12 + 16 * n + 8;
    if names > doff || doff > d.len() {
        return Err("SARC layout out of bounds");
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let base = hl + 12 + 16 * i;
        let attr = u32_le(d, base + 4).ok_or("SFAT attr")?;
        let s = u32_le(d, base + 8).ok_or("SFAT start")? as usize;
        let e = u32_le(d, base + 12).ok_or("SFAT end")? as usize;
        if s > e || e > d.len() - doff {
            return Err("SARC member out of bounds");
        }
        let nm = if (attr >> 24) != 0 {
            let o = names + ((attr & 0x00ff_ffff) as usize) * 4;
            let end = d[o..].iter().position(|&b| b == 0).map(|p| o + p).unwrap_or(d.len());
            String::from_utf8_lossy(&d[o..end]).into_owned()
        } else {
            format!("#{i}")
        };
        out.push((nm, &d[doff + s..doff + e]));
    }
    Ok(out)
}

pub fn bnsh_shaders<'a>(d: &'a [u8], base: usize, label: &str) -> Vec<RawShader<'a>> {
    let mut out = Vec::new();
    if d.get(base + 0x60..base + 0x64) != Some(b"grsc") {
        return out;
    }
    let Some(nvar) = u32_le(d, base + 0x7c) else {
        return out;
    };
    let Some(pvar) = u64_le(d, base + 0x80) else {
        return out;
    };
    for v in 0..nvar as usize {
        let Some(prog) = u64_le(d, base + pvar as usize + 0x40 * v + 0x10) else {
            continue;
        };
        if prog == 0 {
            continue;
        }
        for si in 0..9 {
            let Some(sc) = u64_le(d, base + prog as usize + 8 + 8 * si) else {
                continue;
            };
            if sc == 0 {
                continue;
            }
            let off = base + sc as usize;
            let (Some(pctl), Some(pcode), Some(ncode), Some(nctl)) = (
                u64_le(d, off + 8),
                u64_le(d, off + 16),
                u32_le(d, off + 24),
                u32_le(d, off + 28),
            ) else {
                continue;
            };
            let (ctl_start, ctl_len_v) = (base + pctl as usize, nctl as usize);
            let (code_start, code_len_v) = (base + pcode as usize, ncode as usize);
            let (Some(ctl), Some(code)) = (
                d.get(ctl_start..ctl_start + ctl_len_v),
                d.get(code_start..code_start + code_len_v),
            ) else {
                continue;
            };
            let st: &'static str = if si < 6 {
                let s = STAGES[si];
                if nvn2_ctl(ctl) && ctl.len() > 0x56 {
                    match ctl[0x56] {
                        6 => "ts",
                        7 => "ms",
                        _ => s,
                    }
                } else {
                    s
                }
            } else {
                if !(0x48..0x10000).contains(&ctl_len_v) || !nvn2_ctl(ctl) {
                    continue;
                }
                match nvn2_stage(ctl, code) {
                    Some(s @ ("ts" | "ms")) => s,
                    _ => continue,
                }
            };
            if check_shader(st, ctl, code).is_ok() {
                out.push(RawShader {
                    where_str: format!("{label}#BNSH@{base:#x}#var{v}"),
                    stage: st,
                    ctl,
                    code,
                });
            }
        }
    }
    out
}

pub fn bahs_shaders<'a>(d: &'a [u8], label: &str) -> Vec<RawShader<'a>> {
    let (Some(rsz), Some(region)) = (u32_le(d, 0x18), u32_le(d, 0x1c)) else {
        return Vec::new();
    };
    let (rsz, region) = (rsz as usize, region as usize);
    if region + rsz > d.len() {
        return Vec::new();
    }
    let mut found: Vec<(usize, usize, RawShader<'a>)> = Vec::new();
    let mut a = 0usize;
    while a + 16 <= region {
        let off = u32_le(d, a).unwrap() as usize;
        let z = u32_le(d, a + 4).unwrap();
        let csz = u32_le(d, a + 8).unwrap() as usize;
        let rel = i32_le(d, a + 12).unwrap() as isize;
        let c_isize = a as isize + rel;
        let nvn2 = c_isize >= 0
            && (0x48..0x10000).contains(&csz)
            && (c_isize as usize) + csz <= region
            && nvn2_ctl(&d[c_isize as usize..c_isize as usize + csz]);
        let valid_sz = nvn2 || csz == 0x880 || csz == 0x88c || (0x760..0x880).contains(&csz);
        if z == 0 && valid_sz && off % 0x100 == 0 && c_isize >= 0 {
            let c = c_isize as usize;
            if c + csz <= region && d[c..c + 4] == CTL_MAGIC {
                let n = u32_le(d, c + if nvn2 { 0x44 } else { 0x704 }).unwrap_or(0) as usize;
                if off + n <= rsz {
                    let code = &d[region + off..region + off + n];
                    let ctl = &d[c..c + csz];
                    let st = if nvn2 {
                        nvn2_stage(ctl, code)
                    } else if code.get(..4) == Some(&CS_MAGIC) {
                        Some("cs")
                    } else if code.get(..4) == Some(&GFX_MAGIC) {
                        sph_stage(code)
                    } else {
                        None
                    };
                    if let Some(stage) = st {
                        if check_shader(stage, ctl, code).is_ok() {
                            found.push((
                                a,
                                c,
                                RawShader {
                                    where_str: format!("{label}#BAHS@{a:#x}"),
                                    stage,
                                    ctl,
                                    code,
                                },
                            ));
                        }
                    }
                }
            }
        }
        a += 4;
    }
    let spans: Vec<(usize, usize)> = found
        .iter()
        .map(|(_, c, s)| (*c, *c + ctl_len(s.ctl)))
        .collect();
    found
        .into_iter()
        .filter(|(a_pos, _, _)| !spans.iter().any(|&(lo, hi)| *a_pos >= lo && *a_pos < hi))
        .map(|(_, _, s)| s)
        .collect()
}

pub fn scan_blob_owned(
    d: &[u8],
    label: &str,
    depth: usize,
    out: &mut Vec<OwnedShader>,
) {
    if depth > NEST_LIMIT {
        return;
    }
    if d.starts_with(b"Yaz0") {
        if let Ok(dec) = yaz0_decompress(d) {
            scan_blob_owned(&dec, label, depth + 1, out);
        }
        return;
    }
    if d.starts_with(b"SARC") {
        if let Ok(members) = sarc_members(d) {
            for (nm, m) in members {
                scan_blob_owned(m, &format!("{label}/{nm}"), depth + 1, out);
            }
        }
        return;
    }
    if d.starts_with(b"BAHS") {
        for s in bahs_shaders(d, label) {
            out.push(OwnedShader {
                where_str: s.where_str,
                stage: s.stage,
                ctl: s.ctl.to_vec(),
                code: s.code.to_vec(),
            });
        }
        return;
    }
    // Scan for BNSH\0\0\0\0 occurrences
    let needle = b"BNSH\0\0\0\0";
    let mut pos = 0usize;
    while pos + 0x20 <= d.len() {
        if let Some(found) = d[pos..].windows(8).position(|w| w == needle) {
            let start = pos + found;
            if start + 0x20 <= d.len() {
                let sz = u32_le(d, start + 0x1c).unwrap_or(0) as usize;
                if start + sz <= d.len() && d.get(start + 0x60..start + 0x64) == Some(b"grsc") {
                    for s in bnsh_shaders(d, start, label) {
                        out.push(OwnedShader {
                            where_str: s.where_str,
                            stage: s.stage,
                            ctl: s.ctl.to_vec(),
                            code: s.code.to_vec(),
                        });
                    }
                }
            }
            pos = start + 8;
        } else {
            break;
        }
    }
}

pub struct StreamingPack {
    seen_keys: HashMap<[u8; 8], [u8; 32]>,
    writers: HashMap<String, BufWriter<File>>,
    out_dir: std::path::PathBuf,
    key_skip: (usize, usize),
    pub unique_count: usize,
    pub total_count: usize,
}

impl StreamingPack {
    pub fn new(out_dir: &Path, key_skip: (usize, usize)) -> std::io::Result<Self> {
        std::fs::create_dir_all(out_dir)?;
        Ok(Self {
            seen_keys: HashMap::new(),
            writers: HashMap::new(),
            out_dir: out_dir.to_path_buf(),
            key_skip,
            unique_count: 0,
            total_count: 0,
        })
    }

    pub fn add_shader(
        &mut self,
        stage: &str,
        group: &str,
        ctl: &[u8],
        code: &[u8],
    ) -> Result<bool, String> {
        self.total_count += 1;
        let key = skey(ctl, code, self.key_skip);
        let sig = shader_sig(ctl, code, self.key_skip);
        if let Some(old_sig) = self.seen_keys.get(&key) {
            if old_sig != &sig {
                return Err(format!(
                    "key {} is the same but the control section or code area differs",
                    skey_hex(&key)
                ));
            }
            return Ok(false);
        }
        self.seen_keys.insert(key, sig);
        self.unique_count += 1;
        let fname = format!("{stage}_{group}.bin");
        if !self.writers.contains_key(&fname) {
            let f = File::create(self.out_dir.join(&fname)).map_err(|e| e.to_string())?;
            self.writers.insert(fname.clone(), BufWriter::with_capacity(64 * 1024, f));
        }
        let w = self.writers.get_mut(&fname).unwrap();
        write_package_record(w, ctl, code).map_err(|e| e.to_string())?;
        Ok(true)
    }

    pub fn finish(mut self) -> std::io::Result<(usize, usize)> {
        for (_, mut w) in self.writers.drain() {
            w.flush()?;
        }
        Ok((self.unique_count, self.total_count))
    }
}

/// Scans a list of RomFS file paths in parallel using `memmap2` + `rayon`,
/// streaming deduplicated `BTCDBTCD` records into `out_dir`.
pub fn scan_files_parallel(
    files: &[(String, String)], // (abs_path, rel_label)
    out_dir: &Path,
    key_skip: (usize, usize),
) -> Result<(usize, usize), String> {
    let pack = Mutex::new(StreamingPack::new(out_dir, key_skip).map_err(|e| e.to_string())?);
    let err: Mutex<Option<String>> = Mutex::new(None);

    files.par_iter().for_each(|(abs_path, rel_label)| {
        if err.lock().unwrap().is_some() {
            return;
        }
        let Ok(file) = File::open(abs_path) else {
            return;
        };
        let Ok(mmap) = (unsafe { Mmap::map(&file) }) else {
            return;
        };
        let data: &[u8] = &mmap;

        // Decompress plain zstd files (.zs / .zst) if applicable
        let mut local_shaders = Vec::new();
        if data.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
            if let Ok(dec) = zstd::stream::decode_all(data) {
                scan_blob_owned(&dec, rel_label, 0, &mut local_shaders);
            }
        } else if data.starts_with(b"KPKA") {
            scan_kpka_entries(data, rel_label, &mut local_shaders);
        } else {
            scan_blob_owned(data, rel_label, 0, &mut local_shaders);
        }

        if !local_shaders.is_empty() {
            let mut guard = pack.lock().unwrap();
            for s in local_shaders {
                if let Err(e) = guard.add_shader(s.stage, "other", &s.ctl, &s.code) {
                    *err.lock().unwrap() = Some(e);
                    break;
                }
            }
        }
    });

    if let Some(e) = err.into_inner().unwrap() {
        return Err(e);
    }
    let pack = pack.into_inner().unwrap();
    pack.finish().map_err(|e| e.to_string())
}

fn scan_kpka_entries(d: &[u8], label: &str, out: &mut Vec<OwnedShader>) {
    if d.len() < 16 || &d[..4] != b"KPKA" || d[4] != 4 {
        return;
    }
    let feat = u16_le(d, 6).unwrap_or(0);
    if (feat & !0x20) != 0 {
        return;
    }
    let n = u32_le(d, 8).unwrap_or(0) as usize;
    let table_end = 16 + 0x30 * n;
    if table_end > d.len() {
        return;
    }
    let mut seen_hashes = HashSet::new();
    for i in 0..n {
        let base = 16 + 0x30 * i;
        let path_hash = u64_le(d, base).unwrap_or(0);
        if !seen_hashes.insert(path_hash) {
            continue;
        }
        let off = u64_le(d, base + 8).unwrap_or(0) as usize;
        let csz = u64_le(d, base + 16).unwrap_or(0) as usize;
        let attr = u64_le(d, base + 32).unwrap_or(0);
        if (attr & 0x0100_0000) != 0 || off + csz > d.len() {
            continue;
        }
        let raw = &d[off..off + csz];
        let comp = attr & 0xf;
        match comp {
            0 => scan_blob_owned(raw, &format!("{label}#{path_hash:016x}"), 0, out),
            1 => {
                let mut dec = Vec::new();
                if DeflateDecoder::new(raw).read_to_end(&mut dec).is_ok() {
                    scan_blob_owned(&dec, &format!("{label}#{path_hash:016x}"), 0, out);
                }
            }
            2 => {
                if let Ok(dec) = zstd::stream::decode_all(raw) {
                    scan_blob_owned(&dec, &format!("{label}#{path_hash:016x}"), 0, out);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skey_and_record_format() {
        let mut ctl = vec![0u8; 0x900];
        ctl[..4].copy_from_slice(&CTL_MAGIC);
        ctl[0x20..0x24].copy_from_slice(&(0x880u32).to_le_bytes());
        ctl[0x704..0x708].copy_from_slice(&(0x40u32).to_le_bytes());
        let mut code = vec![0u8; 0x40];
        code[..4].copy_from_slice(&GFX_MAGIC);
        assert!(check_shader("vs", &ctl, &code).is_ok());

        let k = skey(&ctl, &code, (0, 0));
        assert_eq!(skey_hex(&k).len(), 16);

        let mut buf = Vec::new();
        write_package_record(&mut buf, &ctl, &code).unwrap();
        assert_eq!(&buf[..8], b"BTCDBTCD");
        assert_eq!(u32_le(&buf, 0x28), Some(0x900));
        assert_eq!(u32_le(&buf, 0x2c), Some(0x40));
        assert_eq!(buf.len(), 0x38 + 0x900 + 0x40);
    }

    #[test]
    fn yaz0_roundtrip_literal() {
        // Minimal Yaz0: 4 literal bytes "ABCD"
        let mut d = Vec::new();
        d.extend_from_slice(b"Yaz0");
        d.extend_from_slice(&4u32.to_be_bytes());
        d.extend_from_slice(&[0u8; 8]);
        d.push(0b1111_0000);
        d.extend_from_slice(b"ABCD");
        assert_eq!(yaz0_decompress(&d).unwrap(), b"ABCD");
    }
}
