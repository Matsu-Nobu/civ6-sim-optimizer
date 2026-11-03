"""Sums a samply profile by function: the share of samples a function is running in (self) and is on the stack in
(total, once a sample).

  profile_summary.py <profile.json.gz> [--top 40] [--match TEXT]

The profile is one `samply record --save-only --unstable-presymbolicate -o <profile.json.gz> ...` writes; its
symbols are in <profile.json>.syms.json next to it. --match keeps the functions whose name contains TEXT.
"""
import bisect
import collections
import gzip
import json
import re
import sys


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    path = sys.argv[1]
    top = int(sys.argv[sys.argv.index("--top") + 1]) if "--top" in sys.argv else 40
    match = sys.argv[sys.argv.index("--match") + 1] if "--match" in sys.argv else None
    prof = json.load(gzip.open(path))
    syms = json.load(open(re.sub(r"\.gz$", "", path) + ".syms.json"))
    names = syms["string_table"]
    # library debug name -> sorted (rva, size, name)
    tables = {}
    for lib in syms["data"]:
        rows = sorted((x["rva"], x["size"], names[x["symbol"]]) for x in lib["symbol_table"])
        tables[lib["debug_name"]] = ([r[0] for r in rows], rows)
    libs = prof["libs"]

    def symbol(lib_name, address):
        if lib_name not in tables:
            return f"{lib_name}+{address:#x}"
        starts, rows = tables[lib_name]
        k = bisect.bisect_right(starts, address) - 1
        if k >= 0 and address < rows[k][0] + max(rows[k][1], 1):
            return rows[k][2]
        return f"{lib_name}+{address:#x}"

    self_n, total_n = collections.Counter(), collections.Counter()
    samples = 0
    for th in prof["threads"]:
        ft, fn, rt, st = th["frameTable"], th["funcTable"], th["resourceTable"], th["stackTable"]
        frame_name = {}

        def name_of(frame):
            if frame not in frame_name:
                res = fn["resource"][ft["func"][frame]]
                lib = libs[rt["lib"][res]]["debugName"] if res is not None and res >= 0 and rt["lib"][res] is not None else "?"
                frame_name[frame] = symbol(lib, ft["address"][frame])
            return frame_name[frame]
        for stack in th["samples"]["stack"]:
            if stack is None:
                continue
            samples += 1
            seen = set()
            first = True
            while stack is not None:
                n = name_of(st["frame"][stack])
                if first:
                    self_n[n] += 1
                    first = False
                if n not in seen:
                    total_n[n] += 1
                    seen.add(n)
                stack = st["prefix"][stack]
    print(f"{samples} samples")
    for title, counter in (("total (on the stack)", total_n), ("self (running)", self_n)):
        print(f"-- {title}")
        rows = [(n, c) for n, c in counter.most_common() if match is None or match in n][:top]
        for n, c in rows:
            print(f"{100 * c / samples:6.1f}%  {n[:160]}")


if __name__ == "__main__":
    main()
