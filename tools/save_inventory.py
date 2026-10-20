"""Lists what a Civ6 save holds, to find recorded facts before inferring them.

  save_inventory.py datasets <save>              every REPLAYDATASET_* series: (player, turn, value) records
  save_inventory.py strings <save> [prefix]      identifier-like strings in the decompressed save, with counts
  save_inventory.py newstrings <save a> <save b> strings whose count changed between two saves
"""
import collections
import re
import struct
import sys
import zlib


def decompress(path):
    s = open(path, "rb").read()
    fr = max(s.rfind(bytes([0x72, 0xE1, 0x34, 0x30])), 0)
    st = s.find(b"\x78\x9c", fr)
    en = s.rfind(b"\x00\x00\xff\xff")
    d = s[st:en + 4]
    joined = b"".join(d[p:p + 65536] for p in range(0, len(d), 65540))
    return zlib.decompressobj().decompress(joined)


def datasets(p):
    out = {}
    for m in re.finditer(rb"REPLAYDATASET_[A-Z_]+", p):
        name = m.group().decode()
        at = m.start()
        if at < 4 or struct.unpack_from("<I", p, at - 4)[0] != len(name) or name.endswith("_NAME"):
            continue
        q = at + len(name)
        label = struct.unpack_from("<I", p, q)[0]
        if label > 200:
            continue
        q += 4 + label + 5
        kind, count = struct.unpack_from("<II", p, q)
        if kind not in (1, 2) or count > 1_000_000:
            continue
        q += 8
        recs = []
        for _ in range(count):
            pl, turn, v = struct.unpack_from("<iii", p, q)
            if not (0 <= pl <= 63 and 0 <= turn <= 10000):
                break
            recs.append((pl, turn, v / 256))
            q += 12
        out[name] = (kind, recs)
    return out


def strings(p):
    return collections.Counter(m.group().decode() for m in re.finditer(rb"[A-Z][A-Z0-9_]{5,}", p))


cmd = sys.argv[1]
if cmd == "datasets":
    for name, (kind, recs) in sorted(datasets(decompress(sys.argv[2])).items()):
        print(name, "kind", kind, "records", len(recs))
        if len(sys.argv) > 3:
            pl = int(sys.argv[3])
            print("  ", [(t, v) for (q, t, v) in recs if q == pl])
elif cmd == "strings":
    pre = sys.argv[3] if len(sys.argv) > 3 else ""
    for s, n in sorted(strings(decompress(sys.argv[2])).items()):
        if s.startswith(pre):
            print(n, s)
elif cmd == "newstrings":
    a, b = strings(decompress(sys.argv[2])), strings(decompress(sys.argv[3]))
    for s in sorted(set(a) | set(b)):
        if a[s] != b[s]:
            print(a[s], "->", b[s], s)
