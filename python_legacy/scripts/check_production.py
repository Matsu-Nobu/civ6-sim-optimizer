"""Check one-turn production accumulation: progress(T+1) - progress(T) vs production computed from state T.

Usage: python3 python_legacy/scripts/check_production.py <export.jsonl> <rules.sqlite>
For completed items it also prints the cost implied by the overflow next turn and our cost formula.
"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.production import item_completed, item_cost, production_percent  # noqa: E402
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.yields import city_yields  # noqa: E402


def main():
    export, db = sys.argv[1], sys.argv[2]
    rules = Rules(db)
    rows = [json.loads(line) for line in open(export)]
    ann_path = Path(export).with_name(Path(export).name.split("_p")[0] + "_annotations.json")
    ann = json.loads(ann_path.read_text()) if ann_path.exists() else {}
    by_turn = {d["turn"]: d for d in rows}

    def predicted(d, c):
        routes = [(r["origin"], r["destination"]) for r in ann.get("trade_routes", []) if r["from_turn"] <= d["turn"]]
        by_name = {x["name"]: x for x in d["cities"]}
        prod = city_yields(rules, c, d["player"], routes=routes, cities_by_name=by_name)[0]["YIELD_PRODUCTION"]
        # Amenity (happiness) % is not modelled yet: take it from the recorded yields for now.
        rec = d["cityYields"][c["slot"]]
        base, mod = rec["base"]["YIELD_PRODUCTION"], rec["modified"]["YIELD_PRODUCTION"]
        amenity_pct = (mod / base - 1) * 100 if base else 0
        item = c["production"]["current"]
        return prod * (1 + (amenity_pct + production_percent(rules, d["player"], c, item)) / 100)

    awaiting = {}

    def pending_cost_check(name, overflow):
        if name not in awaiting:
            return
        turn, item, total, cost = awaiting.pop(name)
        implied = total - overflow
        mark = "" if cost is not None and abs(implied - cost) < 0.05 else "  <-- differs" if cost is not None else ""
        print(f"T{turn:2d} {name:8s} {item:28s} completed: total {total:g}, implied cost {implied:g}, formula {cost if cost is None else round(cost, 2)}{mark}")

    ok = bad = 0
    pending_overflow = {}  # city -> overflow from an item completed last turn, added this turn
    for d in rows:
        nxt = by_turn.get(d["turn"] + 1)
        if not nxt:
            continue
        for c in d["cities"]:
            c2 = next((x for x in nxt["cities"] if x["name"] == c["name"]), None)
            item = c["production"]["current"]
            if not c2 or not item:
                continue
            pred = predicted(d, c)
            p0 = c["production"]["progress"].get(item, 0)
            carry = pending_overflow.pop(c["name"], None)
            completed = item_completed(item, c, c2, d["player"], nxt["player"])
            if not completed and c2["production"]["current"] != item:
                print(f"T{d['turn']:2d} {c['name']:8s} {item:28s} switched to {c2['production']['current']} (progress {p0:g} kept)")
                continue
            if not completed:
                obs = c2["production"]["progress"].get(item, 0) - p0
                if carry is not None:
                    note = f"  (= {pred:g} + overflow {obs - pred:g} from the previous item)"
                    print(f"T{d['turn']:2d} {c['name']:8s} {item:28s} +{obs:g}{note}")
                    carry_fix = obs - pred
                    pending_cost_check(c["name"], carry_fix)
                    continue
                good = abs(obs - pred) < 0.1
                ok += good; bad += not good
                if not good:
                    print(f"T{d['turn']:2d} {c['name']:8s} {item:28s} progress {p0:6.2f}: +{obs:6.2f} observed, +{pred:6.2f} predicted")
            else:
                cost = item_cost(rules, d["player"], item, d["gameSpeed"])
                awaiting[c["name"]] = (d["turn"], item, p0 + pred, cost)
                pending_overflow[c["name"]] = True
    print(f"accumulation turns: {ok} match, {bad} differ")


if __name__ == "__main__":
    main()
