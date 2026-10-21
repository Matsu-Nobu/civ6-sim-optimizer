"""Reads the luxury amenity table of a Civ6 save: each luxury resource and the cities it gives an amenity to.

  save_luxuries.py <save> [city id ...]

The table is an int32 count followed by that many (city id int32, resource type hash) pairs. Prints every such table
whose resources are all luxuries (with the city ids given, only tables naming one of them).
"""
import re
import sqlite3
import struct
import sys
import zlib

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from save_inventory import decompress  # noqa: E402

DB = "data/rules/gran_colombia_20261019.sqlite"


def luxuries(db):
    con = sqlite3.connect(db)
    names = [r[0] for r in con.execute("select ResourceType from Resources where ResourceClassType='RESOURCECLASS_LUXURY'")]
    return {zlib.crc32(n.encode()) ^ 0xffffffff: n for n in names}


def tables(p, lux, ids=None):
    out = []
    hashes = set(lux)
    for m in re.finditer(rb"(?s).", p[:0]):
        pass
    i = 4
    n = len(p) - 12
    while i < n:
        h = struct.unpack_from("<I", p, i + 4)[0]
        if h in hashes:
            # walk back to the table start: pairs of (city id, luxury hash)
            j = i
            while j - 8 >= 0 and struct.unpack_from("<I", p, j - 4)[0] in hashes:
                j -= 8
            cnt = struct.unpack_from("<i", p, j - 4)[0]
            pairs = []
            k = j
            while k + 8 <= len(p) and struct.unpack_from("<I", p, k + 4)[0] in hashes:
                pairs.append((struct.unpack_from("<i", p, k)[0], lux[struct.unpack_from("<I", p, k + 4)[0]]))
                k += 8
            if cnt == len(pairs) and pairs and (ids is None or any(c in ids for c, _ in pairs)):
                out.append((j - 4, pairs))
            i = max(k, i + 1)
        else:
            i += 1
    return out


if __name__ == "__main__":
    lux = luxuries(DB)
    ids = {int(x) for x in sys.argv[2:]} or None
    for off, pairs in tables(decompress(sys.argv[1]), lux, ids):
        print(off, pairs)
