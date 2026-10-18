"""Check the food store turn by turn against computed surplus (save's food store, not a replay).

Usage: python3 python_legacy/scripts/check_food.py <export.jsonl> <rules.sqlite>
"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.growth import food_added, growth_threshold, housing  # noqa: E402
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.yields import city_yields  # noqa: E402


def main():
    export, db = sys.argv[1], sys.argv[2]
    rules = Rules(db)
    rows = [json.loads(line) for line in open(export)]
    ann_path = Path(export).with_name(Path(export).name.split("_p")[0] + "_annotations.json")
    ann = json.loads(ann_path.read_text()) if ann_path.exists() else {}
    ok = bad = 0
    for a, b in zip(rows, rows[1:]):
        routes = [(r["origin"], r["destination"]) for r in ann.get("trade_routes", []) if r["from_turn"] <= a["turn"]]
        by_name = {c["name"]: c for c in a["cities"]}
        for c in a["cities"]:
            c2 = next((x for x in b["cities"] if x["name"] == c["name"]), None)
            if not c2 or c["foodStore"] is None or c2["foodStore"] is None:
                continue
            food = city_yields(rules, c, a["player"], routes=routes, cities_by_name=by_name)[0]["YIELD_FOOD"]
            rec = a["cityYields"][c["slot"]]
            pmod = rec["modified"]["YIELD_PRODUCTION"] / rec["base"]["YIELD_PRODUCTION"] if rec["base"]["YIELD_PRODUCTION"] else 1
            growth_pct = 8.0 if pmod > 1.05 else 0.0
            add = food_added(rules, c, food, growth_pct)
            need = growth_threshold(rules, c["population"], a["gameSpeed"])
            s0, s1 = c["foodStore"], c2["foodStore"]
            dpop = c2["population"] - c["population"]
            pred = s0 + add
            grew = pred >= need
            pred_after = pred - need if grew else pred
            good = abs(pred_after - s1) < 0.05 and (dpop == (1 if grew else 0) or dpop == (1 if grew else 0) - 1)
            ok += good; bad += not good
            if not good:
                print(f"T{a['turn']:2d} {c['name']:8s} pop{c['population']}->{c2['population']} store {s0:g} + {add:g} (food {food:g}, housing {housing(rules, c):g}, +{growth_pct:g}%) "
                      f"= {pred:g} / need {need} -> predicted {pred_after:g}, save {s1:g}")
    print(f"food turns: {ok} match, {bad} differ")


if __name__ == "__main__":
    main()
