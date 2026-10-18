"""Rule tests: one case per rule fixed. Each runs one recorded turn from its save (civ6opt step: the recorded choices,
one turn) and compares one value with the game's (tests/cases.json: rule, problem, turn, get, expect, game).
Usage: python3 tests/run_cases.py
"""
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt"


def main():
    cases = json.loads((ROOT / "tests/cases.json").read_text())
    cache, failed = {}, 0
    for c in cases:
        key = (c["problem"], c["turn"])
        if key not in cache:
            out = subprocess.run([str(BIN), "step", str(ROOT / c["problem"]), "--turn", str(c["turn"])], capture_output=True, text=True)
            cache[key] = json.loads(out.stdout)
        s = cache[key]
        city = lambda n: next(x for x in s["cities"] if x["name"] == n)
        tile = lambda n, xy: next(t["yields"] for t in city(n)["tiles"] if t["xy"] == xy)
        got = eval(c["get"], {"s": s, "city": city, "tile": tile, "sorted": sorted})
        ok = got == c["expect"]
        failed += not ok
        print(f"{'ok  ' if ok else 'FAIL'} {c['rule']}" + ("" if ok else f": got {got}, game {c['expect']} ({c['game']})"))
    print(f"{len(cases) - failed}/{len(cases)} passed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
