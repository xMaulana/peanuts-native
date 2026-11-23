#!/usr/bin/env python3
# A .nup made before 2026-11-21 without its base package, when its newest update carries the whole game (nximport.c, nximport_set_keep_base).
# An update's package holds a full RomFS, so the base package was a second copy nothing reads to play: a game whose data is one big archive the update
# touches was twice its size (Yoshi's Crafted World 11.34 GB -> 5.67 GB). No dumps and no keys: the other packages' files are copied out of the .nup byte
# for byte (each extent once, its codec and SHA-256 kept), the updates are marked standalone, and the base's icon, title and publisher records move to the
# newest update's meta block (where dev.peanuts.ui.NupIcon looks when there is no base). A .nup whose newest update lacks its own exefs/main, main.npdm or
# RomFS, or that has no update, is left as it is.
#
#   nup_drop_base.py <in.nup> [<out.nup>]   out defaults to in: the original is then renamed <in>.withbase (--delete removes it instead)
#   --dry-run    only say what would be dropped and the size after
#   --verify     also read every file of the result and check its SHA-256
#   --adb <serial> <device path>   the .nup on an Android device: only its superblock, TOC and meta blocks are read over adb; the new header and meta are
#                pushed beside it and the kept byte ranges are copied on the device with toybox dd, each range's SHA-256 compared old against new
#                before the original is replaced (--delete is implied: the device has no room for both)
import argparse, hashlib, os, shlex, struct, subprocess, sys, tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'dm' / 'tools'))
import nup as N   # noqa: E402

SB = struct.Struct('<4sHHQQ32sQQII16s32s'); TOC = struct.Struct('<4sIQQQQQ'); PKG = struct.Struct('<QIIIIBBHIQI20s')
DIR = struct.Struct('<IIIIQ'); FILE = struct.Struct('<IIQQQ32sBBH12s'); TLV = struct.Struct('<HHI')
CHUNK = 64 << 20


def gb(n): return f'{n / 1e9:.2f} GB'


def meta_at(n, i):
    _, _, _, _, _, _, _, _, _, moff, msize, _ = PKG.unpack_from(n.toc, n.o_pkg + PKG.size * i)
    return moff, msize


def tlvs(b):
    out, o = [], 0
    while o + TLV.size <= len(b):
        t, _, sz = TLV.unpack_from(b, o); o += TLV.size
        out.append((t, b[o:o + sz])); o += sz
    return out


def merged_meta(base_meta, upd_meta):
    """The newest update's block with the application's records (icon, title, publisher) of the base's added where it has none"""
    b, u = tlvs(base_meta), tlvs(upd_meta)
    shifted = any(t == 0 for t, _ in b + u)   # the a55uka/nup packer's numbering, one lower (android/NUP.md)
    app = {2, 4, 5} if shifted else {3, 5, 6}
    have = {t for t, _ in u}
    recs = sorted(u + [(t, d) for t, d in b if t in app and t not in have], key=lambda r: r[0])
    return b''.join(TLV.pack(t, 0, len(d)) + d for t, d in recs)


def plan(n):
    """(base package, the packages kept, the newest update) or a reason it cannot be dropped"""
    base = [p for p in n.packages if p.kind == N.BASE]
    ups = [p for p in n.packages if p.kind == N.UPDATE]
    if not base: return 'no base package'
    if not ups: return 'no update: the base is the game'
    top = max(ups, key=lambda p: p.version)
    try: ok = not n.is_dir(n.find('exefs/main', ('pkg', top.index))) and not n.is_dir(n.find('exefs/main.npdm', ('pkg', top.index)))
    except FileNotFoundError: ok = False
    if not ok: return f'{top.selector} has no exefs/main and main.npdm of its own: the base is still needed'
    if top.romfs == N.NONE or not n.children(top.romfs): return f'{top.selector} has no RomFS of its own: the base is still needed'
    return base[0], [p for p in n.packages if p.kind != N.BASE], top


class Layout:
    """The new .nup: head (superblock + TOC, padded to the blob), runs [(old offset, new offset, length)] to copy, the meta bytes at meta_off, size end"""

    def __init__(self, n, keep, top, metas):
        roots = [p.root for p in keep]
        # node ids as nximport.c write_nup gives them: every directory breadth first from the package roots, then the files in directory order. A directory
        # reached twice (shared by two packages) is written twice: new ids are places in this list, not old nodes
        new_dirs, i = list(roots), 0
        while i < len(new_dirs): new_dirs.extend(c for c in n._dir_kids(new_dirs[i]) if c < n.nd); i += 1
        new_files = [c for d in new_dirs for c in n._dir_kids(d) if c >= n.nd]
        nd, nf = len(new_dirs), len(new_files)
        pool, interned = bytearray(), {}

        def intern(b):
            if b not in interned: interned[b] = len(pool); pool.extend(b)
            return interned[b]
        kids, first, next_dir, fi = [], [], len(roots), 0
        for d in new_dirs:
            first.append(len(kids))
            for c in n._dir_kids(d):
                if c < n.nd: kids.append(next_dir); next_dir += 1
                else: kids.append(nd + fi); fi += 1
        o_pk = TOC.size; o_dir = o_pk + PKG.size * len(keep); o_file = o_dir + DIR.size * nd; o_kids = o_file + FILE.size * nf; o_pool = o_kids + 4 * len(kids)
        nm_d = [intern(bytes(n.name(d))) for d in new_dirs]; nm_f = [intern(bytes(n.name(f))) for f in new_files]
        toc_size = o_pool + len(pool); blob = (0x80 + toc_size + 15) & ~15
        # extents: each kept one once, in the old order; one that follows the previous within its 16-byte padding keeps the old spacing, so runs of
        # neighbours are one copy
        olds = sorted({(off, stored) for f in new_files for _, off, stored, _, _ in [n.record(f)] if stored})
        ext, runs, at = {}, [], blob
        for off, stored in olds:
            if runs and off == ((runs[-1][0] + runs[-1][2] + 15) & ~15):
                o0, n0, l0 = runs[-1]; new = n0 + (off - o0); runs[-1] = (o0, n0, off + stored - o0)
            else:
                new = (at + 15) & ~15; runs.append((off, new, stored))
            ext[(off, stored)] = new; at = new + stored
        end = at
        self.meta = b''; mloc = []
        for p, m in zip(keep, metas):
            mloc.append((end + len(self.meta), len(m)) if m else (0, 0)); self.meta += m
        self.meta_off = end; end += len(self.meta)
        toc = bytearray(toc_size)
        TOC.pack_into(toc, 0, b'TOC\x01', len(keep), nd, nf, o_pool, len(pool), 0)
        for i, p in enumerate(keep):
            rk = list(n._dir_kids(p.root))
            nid = lambda old: N.NONE if old == N.NONE else kids[first[i] + rk.index(old)]
            flags = (p.flags | 1) if p.kind == N.UPDATE else p.flags
            PKG.pack_into(toc, o_pk + PKG.size * i, p.title_id, i, nid(p.exefs), nid(p.romfs), p.version, p.kind, flags, 0, 0, mloc[i][0], mloc[i][1], b'')
        for i, d in enumerate(new_dirs): DIR.pack_into(toc, o_dir + DIR.size * i, nm_d[i], len(n.name(d)), first[i], len(n._dir_kids(d)), 0)
        for i, f in enumerate(new_files):
            size, off, stored, sha, codec = n.record(f)
            FILE.pack_into(toc, o_file + FILE.size * i, nm_f[i], len(n.name(f)), size, ext[(off, stored)] if stored else 0, stored, sha, codec, 0, 0, b'')
        struct.pack_into(f'<{len(kids)}I', toc, o_kids, *kids)
        toc[o_pool:] = pool
        u = hashlib.sha256(b''.join(struct.pack('<QIB', p.title_id, p.version, p.kind) for p in keep)).digest()[:16]
        sb = SB.pack(b'NUP\x01', 0x80, 1, 0x80, toc_size, hashlib.sha256(toc).digest(), blob, end, len(keep), 0, u, b'')
        self.head = (sb + bytes(toc)).ljust(blob, b'\0')
        self.runs, self.end = runs, end


def layout_of(n, read):
    """read(offset, size) → bytes of the old .nup. A Layout, or a reason it is kept as it is"""
    r = plan(n)
    if isinstance(r, str): return r
    base, keep, top = r
    rd = lambda i: (lambda o, s: read(o, s) if o and s else b'')(*meta_at(n, i))
    base_meta = rd(base.index)
    metas = [merged_meta(base_meta, rd(p.index)) if p is top else rd(p.index) for p in keep]
    return Layout(n, keep, top, metas), keep


def write_local(n, src, lay, out):
    with open(out, 'wb') as w:
        w.write(lay.head)
        for old, new, ln in lay.runs:
            w.seek(new); done = 0
            while done < ln:
                k = min(CHUNK, ln - done); b = os.pread(src, k, old + done)
                if len(b) != k: raise N.Error(f'{n.path}: short read at {old + done}')
                w.write(b); done += k
        w.seek(lay.meta_off); w.write(lay.meta); w.truncate(lay.end)
        w.flush(); os.fsync(w.fileno())


def tree(n, sel):
    out, stack = {}, [('', n.node(sel))]
    while stack:
        rel, d = stack.pop()
        for name, c in n.children(d):
            if n.is_dir(c): stack.append((f'{rel}/{name}', c))
            else: size, _, _, sha, codec = n.record(c); out[f'{rel}/{name}'] = (size, sha, codec)
    return out


def same_files(a, b, keep):
    """Every file of these packages: the same paths, sizes, codecs and SHA-256 in both"""
    return [(p.selector, p.title_id) for p in b.packages] == [(p.selector, p.title_id) for p in keep] and all(tree(a, p.selector) == tree(b, p.selector) for p in keep)


# ---- a .nup on an Android device
def adb(serial, cmd, **kw): return subprocess.run(['adb', '-s', serial, 'exec-out', cmd], capture_output=True, timeout=kw.get('timeout', 300))


def device_read(serial, path):
    q = shlex.quote(path)
    def read(off, size):
        r = adb(serial, f'dd if={q} bs=4M iflag=skip_bytes,count_bytes skip={off} count={size} status=none')
        if len(r.stdout) != size: raise N.Error(f'{path}: read {len(r.stdout)} of {size} bytes at {off} over adb')
        return r.stdout
    return read


def device_stub(serial, path, tmpdir):
    """A sparse local copy holding only the superblock, the TOC and the meta blocks (the reader's checks pass; extents read as zeros)"""
    read = device_read(serial, path)
    sb = read(0, 0x80); _, _, _, toc_off, toc_size, _, _, fsize, npk, _, _, _ = SB.unpack(sb)
    stub = Path(tmpdir) / 'stub.nup'
    with open(stub, 'wb') as w:
        w.truncate(fsize); w.write(read(0, toc_off + toc_size))
        toc = read(toc_off, toc_size)
        for i in range(npk):
            _, _, _, _, _, _, _, _, _, moff, msize, _ = PKG.unpack_from(toc, TOC.size + PKG.size * i)
            if moff and msize: w.seek(moff); w.write(read(moff, msize))
    return stub, read


def device_script(path, lay, head_p, meta_p):
    O, T, H, M = (shlex.quote(x) for x in (path, path + '.tmp', head_p, meta_p))
    dd = 'dd bs=4M iflag=skip_bytes,count_bytes oflag=seek_bytes conv=notrunc status=none'
    s = ['set -e', f'rm -f {T}', f'dd if={H} of={T} bs=4M status=none']
    for i, (old, new, ln) in enumerate(lay.runs):
        s.append(f'{dd} if={O} of={T} skip={old} seek={new} count={ln}')
        if i % 50 == 0: s.append(f'echo "copy {i + 1}/{len(lay.runs)}"')
    s.append(f'{dd} if={M} of={T} skip=0 seek={lay.meta_off} count={len(lay.meta)}')
    s.append(f'truncate -s {lay.end} {T}')
    s.append(f'[ "$(stat -c %s {T})" = {lay.end} ] || {{ echo "FAIL size"; exit 1; }}')
    s.append(f'[ "$(dd if={T} bs=4M iflag=count_bytes count={len(lay.head)} status=none | sha256sum | cut -d" " -f1)" = {hashlib.sha256(lay.head).hexdigest()} ] || {{ echo "FAIL head"; exit 1; }}')
    for i, (old, new, ln) in enumerate(lay.runs):
        s.append(f'a=$(dd if={O} bs=4M iflag=skip_bytes,count_bytes skip={old} count={ln} status=none | sha256sum); '
                 f'b=$(dd if={T} bs=4M iflag=skip_bytes,count_bytes skip={new} count={ln} status=none | sha256sum); [ "$a" = "$b" ] || {{ echo "FAIL run {i}"; exit 1; }}')
        if i % 50 == 0: s.append(f'echo "check {i + 1}/{len(lay.runs)}"')
    s.append(f'[ "$(dd if={T} bs=4M iflag=skip_bytes,count_bytes skip={lay.meta_off} count={len(lay.meta)} status=none | sha256sum | cut -d" " -f1)" = {hashlib.sha256(lay.meta).hexdigest()} ] || {{ echo "FAIL meta"; exit 1; }}')
    s += [f'mv {T} {O}', f'rm -f {H} {M}', 'echo DONE']
    return '\n'.join(s) + '\n'


def run_device(a):
    serial, path = a.adb
    with tempfile.TemporaryDirectory() as td:
        stub, read = device_stub(serial, path, td)
        n = N.open_nup(stub)
        r = layout_of(n, read)
        name = os.path.basename(path)
        if isinstance(r, str): print(f'{name}: kept as it is ({r})'); return 0
        lay, keep = r
        if a.dry_run: print(f'{name}: {gb(n.size)} -> {gb(lay.end)}, {len(lay.runs)} copies ({", ".join(p.selector for p in keep)})'); return 0
        # the new header checked by the reader before anything is copied (a stub of it: the same sparse trick)
        chk = Path(td) / 'new.nup'
        with open(chk, 'wb') as w: w.truncate(lay.end); w.write(lay.head); w.seek(lay.meta_off); w.write(lay.meta)
        if not same_files(n, N.open_nup(chk), keep): sys.exit(f'[RED] {name}: the new TOC does not hold the same files: nothing done')
        hp, mp, sp = path + '.newhead', path + '.newmeta', path + '.drop_base.sh'
        (Path(td) / 'h').write_bytes(lay.head); (Path(td) / 'm').write_bytes(lay.meta); (Path(td) / 's').write_text(device_script(path, lay, hp, mp))
        for src, dst in (('h', hp), ('m', mp), ('s', sp)):
            r = subprocess.run(['adb', '-s', serial, 'push', str(Path(td) / src), dst], capture_output=True, text=True)
            if r.returncode: sys.exit(f'[RED] {name}: push failed: {r.stderr.strip()}')
    print(f'{name}: {gb(n.size)} -> {gb(lay.end)}, {len(lay.runs)} copies; running on the device', flush=True)
    p = subprocess.run(['adb', '-s', serial, 'shell', f'sh {shlex.quote(sp)}; rc=$?; rm -f {shlex.quote(sp)}; exit $rc'], text=True, capture_output=True)
    last = (p.stdout.strip().splitlines() or [''])[-1]
    if p.returncode or last != 'DONE':
        subprocess.run(['adb', '-s', serial, 'shell', f'rm -f {shlex.quote(path + ".tmp")} {shlex.quote(hp)} {shlex.quote(mp)}'])
        sys.exit(f'[RED] {name}: {last or p.stderr.strip()}: the original is left as it was')
    print(f'{name}: done, {gb(lay.end)}')
    return 0


def main():
    ap = argparse.ArgumentParser(description='drop the base package of a .nup whose newest update carries the whole game')
    ap.add_argument('nup', type=Path, nargs='?'); ap.add_argument('out', type=Path, nargs='?')
    ap.add_argument('--dry-run', action='store_true'); ap.add_argument('--verify', action='store_true'); ap.add_argument('--delete', action='store_true')
    ap.add_argument('--adb', nargs=2, metavar=('SERIAL', 'PATH'))
    a = ap.parse_args()
    if a.adb: return run_device(a)
    if not a.nup: ap.error('a .nup, or --adb')
    src_path = a.nup.resolve()
    n = N.open_nup(src_path)
    fd = os.open(src_path, os.O_RDONLY)
    try:
        r = layout_of(n, lambda o, s: os.pread(fd, s, o))
        if isinstance(r, str): print(f'{src_path.name}: kept as it is ({r})'); return 0
        lay, keep = r
        if a.dry_run: print(f'{src_path.name}: {gb(n.size)} -> {gb(lay.end)}, {len(lay.runs)} copies ({", ".join(p.selector for p in keep)})'); return 0
        out = (a.out or src_path).resolve(); tmp = out.with_name(out.name + '.tmp')
        write_local(n, fd, lay, tmp)
    finally: os.close(fd)
    m = N.open_nup(tmp)   # the reader's own checks (TOC SHA-256, bounds, extents)
    if not same_files(n, m, keep): tmp.unlink(); sys.exit(f'[RED] {src_path.name}: the new .nup does not hold the same files: left as it was')
    if a.verify:
        bad, stack = 0, [m.node(p.selector) for p in keep]
        while stack:
            for _, c in m.children(stack.pop()):
                if m.is_dir(c): stack.append(c)
                elif not m.verify(c): bad += 1
        if bad: tmp.unlink(); sys.exit(f'[RED] {src_path.name}: {bad} files of the new .nup do not match their SHA-256: left as it was')
    kept = ''
    if out == src_path and not a.delete: os.replace(src_path, src_path.with_name(src_path.name + '.withbase')); kept = f'; the original is {src_path.name}.withbase'
    os.replace(tmp, out)
    print(f'{src_path.name}: {gb(n.size)} -> {gb(m.size)} ({", ".join(p.selector for p in keep)}){kept}')
    return 0


if __name__ == '__main__':
    try: sys.exit(main())
    except (N.Error, OSError, subprocess.TimeoutExpired) as e: sys.exit(f'[RED] {e}')
