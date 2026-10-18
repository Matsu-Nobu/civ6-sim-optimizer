"""Compare computed empire yields with the yields recorded in each save.

The yields a save records are the ones produced at the end of the previous turn, so turn T's
recorded yields are compared with the state exported from save T-1.

Usage: python3 python_legacy/scripts/check_yields.py <export.jsonl> <rules.sqlite> [--detail TURN] [--bbcc capital|all|off]
"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.yields import city_yields  # noqa: E402

SHOW = (("F", "YIELD_FOOD"), ("P", "YIELD_PRODUCTION"), ("G", "YIELD_GOLD"), ("S", "YIELD_SCIENCE"), ("C", "YIELD_CULTURE"))


def arg(name, default):
    return sys.argv[sys.argv.index(name) + 1] if name in sys.argv else default


def main():
    export, db = sys.argv[1], sys.argv[2]
    detail = int(arg("--detail", -1))
    bbcc = arg("--bbcc", "capital")
    rules = Rules(db)
    rows = [json.loads(line) for line in open(export)]
    ann_path = Path(export).with_name(Path(export).name.split("_p")[0] + "_annotations.json")
    ann = json.loads(ann_path.read_text()) if ann_path.exists() else {}
    for prev, cur in zip(rows, rows[1:]):
        if not prev["cities"]:
            continue
        total = {k: 0.0 for _, k in SHOW}
        routes = [(r["origin"], r["destination"]) for r in ann.get("trade_routes", []) if r["from_turn"] <= prev["turn"]]
        by_name = {c["name"]: c for c in prev["cities"]}
        for c in prev["cities"]:
            t, parts = city_yields(rules, c, prev["player"], bbcc_mode=bbcc, routes=routes, cities_by_name=by_name)
            for _, k in SHOW:
                total[k] += t[k]
            if detail == cur["turn"]:
                print(f"--- T{cur['turn']} (state of T{prev['turn']}) {c['name']} pop{c['population']}")
                for label, y in parts.items():
                    print(f"   {label:55s}", " ".join(f"{s}{y.get(k, 0):g}" for s, k in SHOW if y.get(k)))
        actual = cur["player"]["yields"]
        cols = []
        for s, k in SHOW:
            a, c = actual.get(k, 0), total[k]
            cols.append(f"{s}{a:g}" + ("" if abs(a - c) < 0.05 else f"({a - c:+.2f})"))
        print(f"T{cur['turn']:2d} cities{len(prev['cities'])} " + " ".join(cols))


if __name__ == "__main__":
    main()
