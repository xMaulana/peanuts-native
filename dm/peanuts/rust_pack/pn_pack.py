"""Optional native Rust acceleration wrapper (`libpn_pack.so`) for SPIR-V
lowering and zstd compression.
"""
from __future__ import annotations

import ctypes
import os
import sys

EXPECTED_ABI = 3


class PnBuf(ctypes.Structure):
    _fields_ = [
        ("ptr", ctypes.POINTER(ctypes.c_uint8)),
        ("len", ctypes.c_size_t),
        ("cap", ctypes.c_size_t),
    ]


def _load():
    names = ["libpn_pack.so"]
    if sys.platform == "darwin":
        names = ["libpn_pack.dylib", "libpn_pack.so"]
    here = os.path.dirname(os.path.abspath(__file__))
    candidates = list(names) + [
        os.path.join(here, "target", "release", n) for n in names
    ]
    for path in candidates:
        try:
            lib = ctypes.CDLL(path)
        except OSError:
            continue
        if lib.pn_pack_abi_version() != EXPECTED_ABI:
            continue
        lib.pn_free_buf.argtypes = [PnBuf]
        lib.pn_free_buf.restype = None
        lib.pn_zstd_compress.argtypes = [ctypes.c_char_p, ctypes.c_size_t, ctypes.c_int32]
        lib.pn_zstd_compress.restype = PnBuf
        lib.pn_spv_open.argtypes = [ctypes.c_char_p, ctypes.c_size_t]
        lib.pn_spv_open.restype = ctypes.c_void_p
        lib.pn_spv_clone.argtypes = [ctypes.c_void_p]
        lib.pn_spv_clone.restype = ctypes.c_void_p
        lib.pn_spv_free.argtypes = [ctypes.c_void_p]
        lib.pn_spv_free.restype = None
        lib.pn_spv_data.argtypes = [ctypes.c_void_p]
        lib.pn_spv_data.restype = PnBuf
        for fn in (
            "pn_spv_strip_unused_caps",
            "pn_spv_formatless_storage_caps",
            "pn_spv_extended_format_caps",
            "pn_spv_workgroup_plain_arrays",
            "pn_spv_precise_minmax",
            "pn_spv_txq_extra_components",
            "pn_spv_buffer_textures_split",
            "pn_spv_nonuniform_arrays",
        ):
            getattr(lib, fn).argtypes = [ctypes.c_void_p]
            getattr(lib, fn).restype = ctypes.c_uint32
        lib.pn_spv_init_outputs.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_uint32), ctypes.c_size_t]
        lib.pn_spv_init_outputs.restype = ctypes.c_uint32
        lib.pn_spv_gl_depth.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
        lib.pn_spv_gl_depth.restype = ctypes.c_int32
        lib.pn_spv_dual_source.argtypes = [ctypes.c_void_p]
        lib.pn_spv_dual_source.restype = ctypes.c_int32
        lib.pn_spv_set_binding.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_uint32]
        lib.pn_spv_set_binding.restype = None
        lib.pn_split_skey.argtypes = [
            ctypes.c_char_p, ctypes.c_size_t,
            ctypes.c_char_p, ctypes.c_size_t,
            ctypes.c_size_t, ctypes.c_size_t,
            ctypes.c_char_p,
        ]
        lib.pn_split_skey.restype = ctypes.c_int32
        lib.pn_split_yaz0.argtypes = [ctypes.c_char_p, ctypes.c_size_t]
        lib.pn_split_yaz0.restype = PnBuf
        lib.pn_split_scan_files.argtypes = [
            ctypes.POINTER(ctypes.c_char_p),
            ctypes.POINTER(ctypes.c_char_p),
            ctypes.c_size_t,
            ctypes.c_char_p,
            ctypes.c_size_t,
            ctypes.c_size_t,
            ctypes.POINTER(ctypes.c_size_t),
            ctypes.POINTER(ctypes.c_size_t),
        ]
        lib.pn_split_scan_files.restype = ctypes.c_int32
        return lib
    return None


LIB = _load()


def _take_buf(buf: PnBuf) -> bytes:
    try:
        if not buf.ptr or buf.len == 0:
            return b""
        return ctypes.string_at(buf.ptr, buf.len)
    finally:
        if LIB is not None:
            LIB.pn_free_buf(buf)


def available() -> bool:
    return LIB is not None


def zstd_compress(data: bytes, level: int = 6) -> bytes:
    if LIB is None:
        raise RuntimeError("libpn_pack not loaded")
    return _take_buf(LIB.pn_zstd_compress(data, len(data), level))


class RustSpvModule:
    """Thin handle wrapper around Rust `SpvModule`."""

    def __init__(self, data: bytes | None = None, *, _handle: int | None = None):
        if LIB is None:
            raise RuntimeError("libpn_pack not loaded")
        if _handle is not None:
            self._h = _handle
        elif data is not None:
            self._h = LIB.pn_spv_open(data, len(data))
            if not self._h:
                raise ValueError("invalid SPIR-V module")
        else:
            raise ValueError("data or _handle required")

    def __del__(self):
        h = getattr(self, "_h", None)
        if h and LIB is not None:
            LIB.pn_spv_free(h)
            self._h = None

    def clone(self) -> "RustSpvModule":
        return RustSpvModule(_handle=LIB.pn_spv_clone(self._h))

    def data(self) -> bytes:
        return _take_buf(LIB.pn_spv_data(self._h))

    def strip_unused_caps(self) -> int:
        return LIB.pn_spv_strip_unused_caps(self._h)

    def formatless_storage_caps(self) -> int:
        return LIB.pn_spv_formatless_storage_caps(self._h)

    def extended_format_caps(self) -> int:
        return LIB.pn_spv_extended_format_caps(self._h)

    def workgroup_plain_arrays(self) -> int:
        return LIB.pn_spv_workgroup_plain_arrays(self._h)

    def precise_minmax(self) -> int:
        return LIB.pn_spv_precise_minmax(self._h)

    def txq_extra_components(self) -> int:
        return LIB.pn_spv_txq_extra_components(self._h)

    def buffer_textures_split(self) -> int:
        return LIB.pn_spv_buffer_textures_split(self._h)

    def nonuniform_arrays(self) -> int:
        return LIB.pn_spv_nonuniform_arrays(self._h)

    def init_outputs(self, extra_locations=()) -> int:
        locs = list(extra_locations)
        if not locs:
            return LIB.pn_spv_init_outputs(self._h, None, 0)
        arr = (ctypes.c_uint32 * len(locs))(*locs)
        return LIB.pn_spv_init_outputs(self._h, arr, len(locs))

    def gl_depth(self, spec_id: int) -> bool:
        return bool(LIB.pn_spv_gl_depth(self._h, spec_id))

    def dual_source(self) -> bool:
        return bool(LIB.pn_spv_dual_source(self._h))

    def set_binding(self, vid: int, s: int, b: int) -> None:
        LIB.pn_spv_set_binding(self._h, vid, s, b)
