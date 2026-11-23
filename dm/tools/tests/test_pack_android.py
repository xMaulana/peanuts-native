"""scripts/pack_android.py's swap of a rewritten module into a pack (the phone's packer updates the user's pack in place): the module's files go in from its
staging folder by rename and its key file comes last, so a pack stopped before the key is written has a module without a key, which the launcher refuses.
Also: the check of the modules' builds against every build row of the profile."""
import os
import sys
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO / 'scripts'))
import pack_android   # noqa: E402


class Swap(unittest.TestCase):
    def test_swap_in_replaces_files_and_writes_the_key_last(self):
        with tempfile.TemporaryDirectory() as d:
            out = Path(d)
            (out / 'main.patched.bin').write_bytes(b'old'); (out / 'main.fixups.tsv').write_text('old\n')
            st = pack_android.staging(out, 'main'); st.mkdir()
            (st / 'main.patched.bin').write_bytes(b'new'); (st / 'main.fixups.tsv').write_text('new\n'); (st / 'build_src.sha256').write_text('x\n')
            seen = []
            real = os.replace
            def spy(a, b): seen.append(Path(b).name); real(a, b)
            pack_android.os.replace = spy
            try: pack_android.swap_in(out, 'main', 'k target=android')
            finally: pack_android.os.replace = real
            self.assertEqual(seen[-1], 'main.android.key', 'the key must go in last')
            self.assertEqual((out / 'main.patched.bin').read_bytes(), b'new')
            self.assertEqual((out / 'main.fixups.tsv').read_text(), 'new\n')
            self.assertEqual((out / 'main.android.key').read_text(), 'k target=android\n')
            self.assertFalse(st.exists())


class Builds(unittest.TestCase):
    def test_every_build_row_counts(self):
        class G:   # a profile with an update's row and the base's, as dm/games/game.py builds() gives them
            def builds(self): return {'v131072': {'display': 'unknown', 'main': 'aa', 'sdk': 'bb'}, 'v0': {'display': 'unknown', 'main': 'cc', 'sdk': 'dd'}}
        self.assertEqual(pack_android.unregistered(G(), {'main': 'aa', 'sdk': 'bb'}), {}, 'the update, named by the first row')
        self.assertEqual(pack_android.unregistered(G(), {'main': 'cc', 'sdk': 'dd'}), {}, 'the base, named by the last row')
        self.assertEqual(pack_android.unregistered(G(), {'main': 'ee', 'subsdk0': 'ff'}), {'main': ['aa', 'cc']}, 'an unknown main; a module no row names is not listed')


class Progress(unittest.TestCase):
    """The phone's rewrite step: a tool run in process passes its progress lines on, placed in the module's share of the step's 1000 (Prepare's bar)"""
    def test_run_in_process_passes_progress_on_in_the_module_share(self):
        import io
        with tempfile.TemporaryDirectory() as d:
            tool = Path(d) / 'tool.py'
            tool.write_text("print('working')\nprint('progress 0 1000 load')\nprint('progress 500 1000 x18', flush=True)\nprint('progress 1000 1000 write')\n")
            out, real = io.StringIO(), sys.stdout
            sys.stdout = out
            try: rc = pack_android.run_in_process(tool, [], {}, Path(d) / 'tool.log', (200, 600))
            finally: sys.stdout = real
            self.assertEqual(rc, 0)
            self.assertEqual(out.getvalue().splitlines(), ['progress 200 1000 code', 'progress 500 1000 code', 'progress 800 1000 code'])
            self.assertIn('working', (Path(d) / 'tool.log').read_text())
            self.assertIn('progress 500 1000 x18', (Path(d) / 'tool.log').read_text(), 'the tool\'s own log keeps its lines')

    def test_nso_text_size(self):
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / 'main'
            f.write_bytes(b'NSO0' + bytes(0x14) + (0x123400).to_bytes(4, 'little') + bytes(0x80))
            self.assertEqual(pack_android.nso_text_size(f), 0x123400)
            f.write_bytes(b'NOPE' + bytes(0x40))
            self.assertEqual(pack_android.nso_text_size(f), 0)


# PEANUTS begin: the in-process rerun (#177)
class Rerun(unittest.TestCase):
    """The phone's packer allows no exec (android/ondevice/dm_pack.c): a tool that would run itself again with a variable (patch_nso.py's DM_PATCH_FRONT, #177)
    asks run_in_process with sys.exit({'rerun_env': ...}) and is run again in process"""
    TOOL = ("import os, sys\n"
            "assert os.environ.get('DM_IN_PROCESS') == '1'\n"
            "if 'DM_PATCH_FRONT' not in os.environ:\n"
            "    print('first pass'); sys.exit({'rerun_env': {'DM_PATCH_FRONT': '0x1d0000'}})\n"
            "print('second pass', os.environ['DM_PATCH_FRONT'])\n")

    def run_tool(self, src):
        with tempfile.TemporaryDirectory() as d:
            tool, log = Path(d) / 'tool.py', Path(d) / 'tool.log'
            tool.write_text(src)
            real = os.execve
            def no_exec(*a, **k): raise OSError('no fork or exec in the packer process (android/ONDEVICE.md)')
            os.execve = no_exec
            try: rc = pack_android.run_in_process(tool, [], {}, log)
            finally: os.execve = real
            return rc, log.read_text()

    def test_rerun_with_the_asked_variable(self):
        rc, text = self.run_tool(self.TOOL)
        self.assertEqual(rc, 0)
        self.assertIn('first pass', text, 'the rerun appends to the first run\'s log')
        self.assertIn('second pass 0x1d0000', text)
        self.assertNotIn('DM_PATCH_FRONT', os.environ)
        self.assertNotIn('DM_IN_PROCESS', os.environ)

    def test_a_tool_that_asks_again_for_the_same_fails(self):
        rc, _ = self.run_tool("import sys\nsys.exit({'rerun_env': {'X': '1'}})\n")
        self.assertEqual(rc, 1)

    def test_patch_nso_asks_instead_of_exec_in_process(self):
        src = (REPO / 'dm/tools/patch_nso.py').read_text()
        at = src.index("rewriting again with {need:#x}")
        block = src[at:at + 800]
        self.assertLess(block.index("os.environ.get('DM_IN_PROCESS')"), block.index('os.execve'), 'in process it must ask for the rerun before any exec')
        self.assertIn("sys.exit({'rerun_env': {'DM_PATCH_FRONT': hex(need)}})", block)
# PEANUTS end


if __name__ == '__main__':
    unittest.main()
