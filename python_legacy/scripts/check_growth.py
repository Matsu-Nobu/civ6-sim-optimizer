"""Replay each city's food store from founding (store 0) and compare growth turns with the saves.

Usage: python3 python_legacy/scripts/check_growth.py <export.jsonl> <rules.sqlite>
Food per turn is computed from each save's state; amenity growth % is read from the recorded
yield modifiers (8% when 'Happy') until amenities are modelled.
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
    store = {}
    for a, b in zip(rows, rows[1:]):
        routes = [(r["origin"], r["destination"]) for r in ann.get("trade_routes", []) if r["from_turn"] <= a["turn"]]
        by_name = {c["name"]: c for c in a["cities"]}
        for c in a["cities"]:
            c2 = next((x for x in b["cities"] if x["name"] == c["name"]), None)
            if not c2:
                continue
            food = city_yields(rules, c, a["player"], routes=routes, cities_by_name=by_name)[0]["YIELD_FOOD"]
            rec = a["cityYields"][c["slot"]]
            pmod = rec["modified"]["YIELD_PRODUCTION"] / rec["base"]["YIELD_PRODUCTION"] if rec["base"]["YIELD_PRODUCTION"] else 1
            growth_pct = 8.0 if pmod > 1.05 else 0.0  # 'Happy' gives +8% non-food yields and +8% growth
            s = store.get(c["name"], 0.0) + food_added(rules, c, food, growth_pct)
            need = growth_threshold(rules, c["population"], a["gameSpeed"])
            grew_pred = s >= need
            dpop = c2["population"] - c["population"]
            settler_done = (b["player"]["unitsTrained"].get("UNIT_SETTLER", 0) > a["player"]["unitsTrained"].get("UNIT_SETTLER", 0)
                            and c["production"]["current"] == "UNIT_SETTLER" and c2["production"]["progress"].get("UNIT_SETTLER", 0) == 0)
            grew_obs = dpop + (1 if settler_done else 0) > 0
            flag = "" if grew_pred == grew_obs else "   <-- differs"
            if grew_pred or grew_obs or flag:
                print(f"T{a['turn']:2d} {c['name']:8s} pop{c['population']} food {food:g} housing {housing(rules, c):g} "
                      f"store {s:6.2f}/{need:6.2f} predicted grow={grew_pred} observed dpop={dpop:+d}{' (settler -1)' if settler_done else ''}{flag}")
            if grew_pred:
                s -= need
            store[c["name"]] = s


if __name__ == "__main__":
    main()
