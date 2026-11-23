"""Byte-for-byte parity tests comparing Rust `libpn_pack` against `vk_android_spv.py`."""
import os
import struct
import sys
import unittest

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO, "dm", "gfx", "tools"))
sys.path.insert(0, os.path.join(REPO, "dm", "peanuts", "rust_pack"))

import pn_pack
import vk_android_spv as py_spv


def _lit(s: str) -> list[int]:
    return py_spv.lit_string(s)


def _pack_words(words: list[int]) -> bytes:
    return struct.pack(f"<{len(words)}I", *words)


def _make_sample_spv() -> bytes:
    glsl = _lit("GLSL.std.450")
    main_name = _lit("main")
    words = [
        py_spv.MAGIC, 0x0001_0000, 0, 20, 0,
        (2 << 16) | py_spv.OP_CAPABILITY, 1,
        (2 << 16) | py_spv.OP_CAPABILITY, py_spv.CAP_FLOAT64,
        (2 + len(glsl)) << 16 | py_spv.OP_EXT_INST_IMPORT, 1, *glsl,
        (3 << 16) | 14, 0, 1,
        (4 + len(main_name)) << 16 | py_spv.OP_ENTRY_POINT, 0, 2, *main_name, 3,
        (4 << 16) | py_spv.OP_DECORATE, 3, py_spv.DEC_LOCATION, 0,
        (2 << 16) | py_spv.OP_TYPE_VOID, 4,
        (3 << 16) | 33, 5, 4,
        (3 << 16) | py_spv.OP_TYPE_FLOAT, 6, 32,
        (4 << 16) | py_spv.OP_TYPE_VECTOR, 7, 6, 4,
        (4 << 16) | py_spv.OP_TYPE_POINTER, 8, py_spv.SC_OUTPUT, 7,
        (4 << 16) | py_spv.OP_VARIABLE, 8, 3, py_spv.SC_OUTPUT,
        (5 << 16) | py_spv.OP_FUNCTION, 4, 2, 0, 5,
        (2 << 16) | py_spv.OP_LABEL, 9,
        (7 << 16) | py_spv.OP_EXT_INST, 6, 10, 1, 37, 11, 12,
        (1 << 16) | py_spv.OP_RETURN,
        (1 << 16) | py_spv.OP_FUNCTION_END,
    ]
    return _pack_words(words)


@unittest.skipUnless(pn_pack.available(), "libpn_pack shared library not built")
class TestRustSpvParity(unittest.TestCase):
    def test_abi_and_zstd(self):
        self.assertEqual(pn_pack.LIB.pn_pack_abi_version(), pn_pack.EXPECTED_ABI)
        compressed = pn_pack.zstd_compress(b"SPIRV" * 200, level=6)
        self.assertGreater(len(compressed), 0)
        self.assertLess(len(compressed), 100)

    def test_passes_byte_exact(self):
        raw = _make_sample_spv()

        pm = py_spv.Module(raw)
        py_spv.strip_unused_caps(pm)
        py_spv.precise_minmax(pm)
        py_spv.init_outputs(pm, [1])
        py_out = pm.data()

        rm = pn_pack.RustSpvModule(raw)
        rm.strip_unused_caps()
        rm.precise_minmax()
        rm.init_outputs([1])
        rs_out = rm.data()

        self.assertEqual(rs_out, py_out)


if __name__ == "__main__":
    unittest.main()
