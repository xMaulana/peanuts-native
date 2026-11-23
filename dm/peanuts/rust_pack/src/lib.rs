//! Native Rust acceleration library (`libpn_pack.so`) for on-device
//! shader splitting and SPIR-V post-processing on Android.

pub mod spv;

use std::ffi::c_void;
use std::ptr;
use std::slice;

use spv::SpvModule;

/// ABI version returned by `pn_pack_abi_version`.
pub const PN_PACK_ABI_VERSION: u32 = 2;

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
