//! Native Rust acceleration library (`libpn_pack.so`) for on-device
//! shader splitting and SPIR-V post-processing on Android.

pub mod split;
pub mod spv;

use std::ffi::{c_char, c_void, CStr};
use std::path::Path;
use std::ptr;
use std::slice;

use spv::SpvModule;

/// ABI version returned by `pn_pack_abi_version`.
pub const PN_PACK_ABI_VERSION: u32 = 3;

#[no_mangle]
pub extern "C" fn pn_pack_abi_version() -> u32 {
    PN_PACK_ABI_VERSION
}

#[repr(C)]
pub struct PnBuf {
    pub ptr: *mut u8,
    pub len: usize,
    pub cap: usize,
}

impl PnBuf {
    fn from_vec(mut v: Vec<u8>) -> Self {
        let ptr = v.as_mut_ptr();
        let len = v.len();
        let cap = v.capacity();
        std::mem::forget(v);
        Self { ptr, len, cap }
    }
}

#[no_mangle]
pub unsafe extern "C" fn pn_free_buf(buf: PnBuf) {
    if !buf.ptr.is_null() && buf.cap > 0 {
        drop(Vec::from_raw_parts(buf.ptr, buf.len, buf.cap));
    }
}

/// Compresses `data[..len]` with zstd at `level` (returns empty PnBuf on error).
#[no_mangle]
pub unsafe extern "C" fn pn_zstd_compress(
    data: *const u8,
    len: usize,
    level: i32,
) -> PnBuf {
    if data.is_null() {
        return PnBuf {
            ptr: ptr::null_mut(),
            len: 0,
            cap: 0,
        };
    }
    let src = slice::from_raw_parts(data, len);
    match zstd::bulk::compress(src, level) {
        Ok(v) => PnBuf::from_vec(v),
        Err(_) => PnBuf {
            ptr: ptr::null_mut(),
            len: 0,
            cap: 0,
        },
    }
}

/// Parses a SPIR-V binary into an opaque `SpvModule` handle. Returns NULL on error.
#[no_mangle]
pub unsafe extern "C" fn pn_spv_open(data: *const u8, len: usize) -> *mut c_void {
    if data.is_null() {
        return ptr::null_mut();
    }
    let bytes = slice::from_raw_parts(data, len);
    match SpvModule::parse(bytes) {
        Ok(m) => Box::into_raw(Box::new(m)) as *mut c_void,
        Err(_) => ptr::null_mut(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn pn_spv_clone(handle: *const c_void) -> *mut c_void {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let m = &*(handle as *const SpvModule);
    Box::into_raw(Box::new(m.clone())) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn pn_spv_free(handle: *mut c_void) {
    if !handle.is_null() {
        drop(Box::from_raw(handle as *mut SpvModule));
    }
}

#[no_mangle]
pub unsafe extern "C" fn pn_spv_data(handle: *const c_void) -> PnBuf {
    if handle.is_null() {
        return PnBuf {
            ptr: ptr::null_mut(),
            len: 0,
            cap: 0,
        };
    }
    let m = &*(handle as *const SpvModule);
    PnBuf::from_vec(m.to_bytes())
}

#[no_mangle]
pub unsafe extern "C" fn pn_spv_strip_unused_caps(handle: *mut c_void) -> u32 {
    if handle.is_null() {
        return 0;
    }
    let m = &mut *(handle as *mut SpvModule);
    m.strip_unused_caps().len() as u32
}

#[no_mangle]
pub unsafe extern "C" fn pn_spv_formatless_storage_caps(handle: *mut c_void) -> u32 {
    if handle.is_null() {
        return 0;
    }
    let m = &mut *(handle as *mut SpvModule);
    m.formatless_storage_caps().len() as u32
}

#[no_mangle]
pub unsafe extern "C" fn pn_spv_extended_format_caps(handle: *mut c_void) -> u32 {
    if handle.is_null() {
        return 0;
    }
    let m = &mut *(handle as *mut SpvModule);
    m.extended_format_caps().len() as u32
}

#[no_mangle]
pub unsafe extern "C" fn pn_spv_workgroup_plain_arrays(handle: *mut c_void) -> u32 {
    if handle.is_null() {
        return 0;
    }
    let m = &mut *(handle as *mut SpvModule);
    m.workgroup_plain_arrays()
}

#[no_mangle]
pub unsafe extern "C" fn pn_spv_precise_minmax(handle: *mut c_void) -> u32 {
    if handle.is_null() {
        return 0;
    }
    let m = &mut *(handle as *mut SpvModule);
    m.precise_minmax()
}

#[no_mangle]
pub unsafe extern "C" fn pn_spv_txq_extra_components(handle: *mut c_void) -> u32 {
    if handle.is_null() {
        return 0;
    }
    let m = &mut *(handle as *mut SpvModule);
    m.txq_extra_components()
}

#[no_mangle]
pub unsafe extern "C" fn pn_spv_buffer_textures_split(handle: *mut c_void) -> u32 {
    if handle.is_null() {
        return 0;
    }
    let m = &mut *(handle as *mut SpvModule);
    m.buffer_textures_split()
}

#[no_mangle]
pub unsafe extern "C" fn pn_spv_nonuniform_arrays(handle: *mut c_void) -> u32 {
    if handle.is_null() {
        return 0;
    }
    let m = &mut *(handle as *mut SpvModule);
    m.nonuniform_arrays()
}

#[no_mangle]
pub unsafe extern "C" fn pn_spv_init_outputs(
    handle: *mut c_void,
    extra_locs: *const u32,
    n_extra: usize,
) -> u32 {
    if handle.is_null() {
        return 0;
    }
    let m = &mut *(handle as *mut SpvModule);
    let extra = if extra_locs.is_null() || n_extra == 0 {
        &[]
    } else {
        slice::from_raw_parts(extra_locs, n_extra)
    };
    m.init_outputs(extra).len() as u32
}

#[no_mangle]
pub unsafe extern "C" fn pn_spv_gl_depth(handle: *mut c_void, spec_id: u32) -> i32 {
    if handle.is_null() {
        return 0;
    }
    let m = &mut *(handle as *mut SpvModule);
    if m.gl_depth(spec_id) { 1 } else { 0 }
}

#[no_mangle]
pub unsafe extern "C" fn pn_spv_dual_source(handle: *mut c_void) -> i32 {
    if handle.is_null() {
        return 0;
    }
    let m = &mut *(handle as *mut SpvModule);
    if m.dual_source() { 1 } else { 0 }
}

#[no_mangle]
pub unsafe extern "C" fn pn_spv_set_binding(
    handle: *mut c_void,
    vid: u32,
    set: u32,
    binding: u32,
) {
    if !handle.is_null() {
        let m = &mut *(handle as *mut SpvModule);
        m.set_binding(vid, set, binding);
    }
}

/// Computes the 16-hex-char shader key (`skey`) into `out_hex[..16]`. Returns 0 on success.
#[no_mangle]
pub unsafe extern "C" fn pn_split_skey(
    ctl: *const u8,
    ctl_len_v: usize,
    code: *const u8,
    code_len_v: usize,
    skip_off: usize,
    skip_len: usize,
    out_hex: *mut u8,
) -> i32 {
    if ctl.is_null() || code.is_null() || out_hex.is_null() {
        return -1;
    }
    let ctl_s = slice::from_raw_parts(ctl, ctl_len_v);
    let code_s = slice::from_raw_parts(code, code_len_v);
    let key = split::skey(ctl_s, code_s, (skip_off, skip_len));
    let hex = split::skey_hex(&key);
    ptr::copy_nonoverlapping(hex.as_ptr(), out_hex, 16);
    0
}

/// Decompresses a Yaz0 buffer into a `PnBuf`.
#[no_mangle]
pub unsafe extern "C" fn pn_split_yaz0(data: *const u8, len: usize) -> PnBuf {
    if data.is_null() {
        return PnBuf {
            ptr: ptr::null_mut(),
            len: 0,
            cap: 0,
        };
    }
    let src = slice::from_raw_parts(data, len);
    match split::yaz0_decompress(src) {
        Ok(v) => PnBuf::from_vec(v),
        Err(_) => PnBuf {
            ptr: ptr::null_mut(),
            len: 0,
            cap: 0,
        },
    }
}

/// Decompresses a CRI CRILAYLA buffer into a `PnBuf` without intermediate bit-string allocation.
#[no_mangle]
pub unsafe extern "C" fn pn_split_crilayla(data: *const u8, len: usize) -> PnBuf {
    if data.is_null() {
        return PnBuf {
            ptr: ptr::null_mut(),
            len: 0,
            cap: 0,
        };
    }
    let src = slice::from_raw_parts(data, len);
    match split::crilayla_decompress(src) {
        Ok(v) => PnBuf::from_vec(v),
        Err(_) => PnBuf {
            ptr: ptr::null_mut(),
            len: 0,
            cap: 0,
        },
    }
}

/// Unmasks a CRI `@UTF` table buffer into a `PnBuf`.
#[no_mangle]
pub unsafe extern "C" fn pn_split_cpk_unmask(data: *const u8, len: usize) -> PnBuf {
    if data.is_null() {
        return PnBuf {
            ptr: ptr::null_mut(),
            len: 0,
            cap: 0,
        };
    }
    let src = slice::from_raw_parts(data, len);
    PnBuf::from_vec(split::cpk_unmask(src))
}

/// Scans `n_files` pairs of `(abs_path, rel_label)` in parallel with `memmap2` + `rayon`
/// and streams deduplicated `BTCDBTCD` package records to `out_dir`.
/// Writes `(unique_count, total_count)` to `out_unique` / `out_total` if non-null.
/// Returns 0 on success, -1 on error.
#[no_mangle]
pub unsafe extern "C" fn pn_split_scan_files(
    abs_paths: *const *const c_char,
    rel_labels: *const *const c_char,
    n_files: usize,
    out_dir: *const c_char,
    skip_off: usize,
    skip_len: usize,
    out_unique: *mut usize,
    out_total: *mut usize,
) -> i32 {
    if abs_paths.is_null() || rel_labels.is_null() || out_dir.is_null() {
        return -1;
    }
    let abs_slice = slice::from_raw_parts(abs_paths, n_files);
    let rel_slice = slice::from_raw_parts(rel_labels, n_files);
    let mut pairs = Vec::with_capacity(n_files);
    for i in 0..n_files {
        if abs_slice[i].is_null() || rel_slice[i].is_null() {
            return -1;
        }
        let a = CStr::from_ptr(abs_slice[i]).to_string_lossy().into_owned();
        let r = CStr::from_ptr(rel_slice[i]).to_string_lossy().into_owned();
        pairs.push((a, r));
    }
    let out_path = Path::new(CStr::from_ptr(out_dir).to_str().unwrap_or(""));
    match split::scan_files_parallel(&pairs, out_path, (skip_off, skip_len)) {
        Ok((u, t)) => {
            if !out_unique.is_null() {
                *out_unique = u;
            }
            if !out_total.is_null() {
                *out_total = t;
            }
            0
        }
        Err(_) => -1,
    }
}
