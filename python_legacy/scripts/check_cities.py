"""Compare computed yields with the per-city yields recorded in each save (same turn, before % modifiers).

Usage: python3 python_legacy/scripts/check_cities.py <export.jsonl> <rules.sqlite> [--bbcc capital|all|off] [--city NAME] [--detail TURN]
Only lines with a mismatch are printed unless --all is given.
"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.yields import city_yields  # noqa: E402

SHOW = (("F", "YIELD_FOOD"), ("P", "YIELD_PRODUCTION"), ("G", "YIELD_GOLD"), ("S", "YIELD_SCIENCE"),
        ("C", "YIELD_CULTURE"), ("R", "YIELD_FAITH"))


def arg(name, default):
    return sys.argv[sys.argv.index(name) + 1] if name in sys.argv else default


def main():
    export, db = sys.argv[1], sys.argv[2]
    bbcc, only_city, detail = arg("--bbcc", "capital"), arg("--city", None), int(arg("--detail", -1))
    show_all = "--all" in sys.argv
    rules = Rules(db)
    ann_path = Path(export).with_name(Path(export).name.split("_p")[0] + "_annotations.json")
    ann = json.loads(ann_path.read_text()) if ann_path.exists() else {}
    for line in open(export):
        d = json.loads(line)
        routes = [(r["origin"], r["destination"]) for r in ann.get("trade_routes", []) if r["from_turn"] <= d["turn"]]
        by_name = {c["name"]: c for c in d["cities"]}
        for c in d["cities"]:
            if only_city and c["name"] != only_city:
                continue
            rec = d["cityYields"][c["slot"]] if c["slot"] < len(d["cityYields"]) else None
            if rec is None:
                print(f"T{d['turn']:2d} {c['name']:8s} no recorded yields")
                continue
            t, parts = city_yields(rules, c, d["player"], bbcc_mode=bbcc, routes=routes, cities_by_name=by_name)
            base, mod = rec["base"], rec["modified"]
            diffs = {k: base[k] - t[k] for _, k in SHOW}
            bad = any(abs(v) > 0.05 for v in diffs.values())
            pct = {s: f"{(mod[k] / base[k] - 1) * 100:+.0f}%" for s, k in SHOW if base[k] and abs(mod[k] - base[k]) > 0.01}
            if bad or show_all or detail == d["turn"]:
                cols = " ".join(f"{s}{base[k]:g}" + ("" if abs(diffs[k]) <= 0.05 else f"({diffs[k]:+.2f})") for s, k in SHOW)
                print(f"T{d['turn']:2d} {c['name']:8s} pop{c['population']} {cols}" + (f"  modifiers {pct}" if pct else ""))
            if detail == d["turn"]:
                for label, y in parts.items():
                    print(f"      {label:55s}", " ".join(f"{s}{y.get(k, 0):g}" for s, k in SHOW if y.get(k)))


if __name__ == "__main__":
    main()
