//! Native Rust acceleration library (`libpn_pack.so`) for on-device
//! shader splitting and SPIR-V post-processing on Android.

/// ABI version returned by `pn_pack_abi_version`.
pub const PN_PACK_ABI_VERSION: u32 = 1;

/// Returns the ABI version of `libpn_pack.so` so Python callers can verify
/// the loaded shared library matches expectations.
#[no_mangle]
pub extern "C" fn pn_pack_abi_version() -> u32 {
    PN_PACK_ABI_VERSION
}
