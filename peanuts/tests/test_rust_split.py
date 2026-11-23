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

    def test_crilayla_and_unmask_match_python_cpk(self):
        sys.path.insert(0, os.path.join(REPO, "dm", "gfx", "tools"))
        import cpk

        # 1. Test @UTF unmask parity
        sample = bytes((i * 73) & 0xFF for i in range(512))
        old_pn = cpk._pn_pack
        try:
            cpk._pn_pack = None
            py_unmasked = cpk.unmask(sample)
        finally:
            cpk._pn_pack = old_pn
        self.assertEqual(pn_pack.cpk_unmask(sample), py_unmasked)

        # 2. Construct a CRILAYLA stream with literals and backreferences (including extended lengths)
        bits = []
        # Output is written backwards from the end:
        # First emit 4 literal bytes at the end: 'Z', 'Y', 'X', 'W'
        for ch in (b"Z"[0], b"Y"[0], b"X"[0], b"W"[0]):
            bits.append("0")
            bits.append(format(ch, "08b"))
        # Next emit a backreference of length 3 + 3 + 7 + 5 = 18 with offset back=3 (repeating 'W','X','Y')
        bits.append("1")
        bits.append(format(3 - 3, "013b"))  # back = 3
        bits.append("11")                   # v = 3 (+3)
        bits.append("111")                  # v = 7 (+7)
        bits.append(format(5, "05b"))       # v = 5 (+5) -> total ln = 18
        bit_str = "".join(bits)
        while len(bit_str) % 8 != 0:
            bit_str += "0"
        # Pack bits in reverse byte order (since CRILAYLA reads from end of compressed stream backwards)
        rev_bytes = [int(bit_str[i:i + 8], 2) for i in range(0, len(bit_str), 8)]
        comp_bytes = bytes(reversed(rev_bytes))
        prefix_0x100 = bytes(i & 0xFF for i in range(0x100))
        usz = 4 + 18
        stream = b"CRILAYLA" + struct.pack("<II", usz, len(comp_bytes)) + comp_bytes + prefix_0x100

        try:
            cpk._pn_pack = None
            py_dec = cpk.crilayla(stream)
        finally:
            cpk._pn_pack = old_pn
        rust_dec = pn_pack.crilayla_decompress(stream)
        self.assertEqual(rust_dec, py_dec)
        self.assertEqual(rust_dec[:0x100], prefix_0x100)
        self.assertEqual(rust_dec[-4:], b"WXYZ")

    def test_vk_android_build_borrow_key(self):
        # Verify vk_android_build.borrow_key() provides _pn_pack to shader_split.skey()
        import ast
        vb_path = os.path.join(REPO, "dm", "gfx", "tools", "vk_android_build.py")
        src = open(vb_path, encoding="utf-8").read()
        tree = ast.parse(src, vb_path)
        prefix = ast.Module(
            body=[n for n in tree.body if not (isinstance(n, ast.If) and "__name__" in ast.dump(n.test))],
            type_ignores=[],
        )
        ns = {"__file__": vb_path, "__name__": "vk_android_build_test"}
        # Extract up to borrow_key without running script-only guard
        lines = src.splitlines()
        guard_line = next(i for i, ln in enumerate(lines) if ln.startswith("if __name__ != '__main__':"))
        exec(compile("\n".join(lines[:guard_line]), vb_path, "exec"), ns)
        ctl, code = _make_dummy_shader()
        expected = hashlib.sha256(ctl[:0x880] + code).hexdigest()[:16]
        skey_fn = ns["borrow_key"]((0, 0))
        self.assertEqual(skey_fn(ctl, code), expected)

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
