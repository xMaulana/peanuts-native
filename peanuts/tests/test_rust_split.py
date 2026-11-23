"""Byte-for-byte parity tests comparing Rust `split` functions against `shader_split.py`."""
import hashlib
import os
import struct
import tempfile
import unittest
import ctypes

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import sys
sys.path.insert(0, os.path.join(REPO, "dm", "peanuts", "rust_pack"))

import pn_pack


def _make_dummy_shader():
    ctl = bytearray(0x900)
    ctl[0:4] = b"\x34\x12\x76\x98"
    struct.pack_into("<I", ctl, 0x20, 0x880)
    struct.pack_into("<I", ctl, 0x704, 0x40)
    code = bytearray(0x40)
    code[0:4] = b"\x78\x56\x34\x12"
    struct.pack_into("<I", code, 0x30, 1 << 10)  # vertex SPH
    return bytes(ctl), bytes(code)


@unittest.skipUnless(pn_pack.available(), "libpn_pack shared library not built")
class TestRustSplitParity(unittest.TestCase):
    def test_skey_matches_python_sha256(self):
        ctl, code = _make_dummy_shader()
        expected = hashlib.sha256(ctl[:0x880] + code).hexdigest()[:16]
        out = ctypes.create_string_buffer(16)
        rc = pn_pack.LIB.pn_split_skey(ctl, len(ctl), code, len(code), 0, 0, out)
        self.assertEqual(rc, 0)
        self.assertEqual(out.raw.decode("ascii"), expected)

    def test_yaz0_decompress(self):
        yaz0 = b"Yaz0" + struct.pack(">I", 4) + b"\x00" * 8 + bytes([0xF0]) + b"ABCD"
        buf = pn_pack.LIB.pn_split_yaz0(yaz0, len(yaz0))
        self.assertEqual(pn_pack._take_buf(buf), b"ABCD")

    def test_parallel_scan_sarc_bahs(self):
        ctl, code = _make_dummy_shader()
        # Build a minimal BAHS container holding 1 shader
        region = 0x1000
        bahs = bytearray(region + len(code))
        bahs[0:4] = b"BAHS"
        struct.pack_into("<II", bahs, 0x18, len(code), region)
        c_off = 0x100
        bahs[c_off:c_off + len(ctl)] = ctl
        bahs[region:region + len(code)] = code
        # Entry at 0x20: off=0, z=0, csz=0x880, rel=c_off - 0x20
        struct.pack_into("<IIIi", bahs, 0x20, 0, 0, 0x880, c_off - 0x20)

        with tempfile.TemporaryDirectory() as tmp:
            in_file = os.path.join(tmp, "test.sharcb")
            with open(in_file, "wb") as f:
                f.write(bahs)
            out_dir = os.path.join(tmp, "pkg")

            abs_arr = (ctypes.c_char_p * 1)(in_file.encode())
            rel_arr = (ctypes.c_char_p * 1)(b"test.sharcb")
            u, t = ctypes.c_size_t(), ctypes.c_size_t()
            rc = pn_pack.LIB.pn_split_scan_files(
                abs_arr, rel_arr, 1, out_dir.encode(), 0, 0, ctypes.byref(u), ctypes.byref(t)
            )
            self.assertEqual(rc, 0)
            self.assertEqual(u.value, 1)
            self.assertEqual(t.value, 1)

            pkg_file = os.path.join(out_dir, "vs_other.bin")
            self.assertTrue(os.path.isfile(pkg_file))
            with open(pkg_file, "rb") as f:
                data = f.read()
            self.assertEqual(data[:8], b"BTCDBTCD")
            self.assertEqual(len(data), 0x38 + len(code) + 0x880)


if __name__ == "__main__":
    unittest.main()
