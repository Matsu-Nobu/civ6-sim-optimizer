"""Check tech and civic accumulation turn by turn, and back out costs from completions.

The save's "current research/civic" is unreliable early on, so the item being researched is taken
from which progress entry grew (or which item completed). Boosts (eureka / inspiration) add a share
of the cost at once. Overflow from a completion shows up on the next item one turn later.

Usage: python3 python_legacy/scripts/check_research.py <export.jsonl> <rules.sqlite> tech|civic
"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.yields import city_yields  # noqa: E402

KEYS = {
    "tech": ("techs", "techProgress", "techsBoosted", "YIELD_SCIENCE", "Technologies", "TechnologyType"),
    "civic": ("civics", "civicProgress", "civicsInspired", "YIELD_CULTURE", "Civics", "CivicType"),
}


def main():
    export, db, which = sys.argv[1], sys.argv[2], sys.argv[3]
    done_key, prog_key, boost_key, yield_key, table, col = KEYS[which]
    rules = Rules(db)
    rows = [json.loads(line) for line in open(export)]
    ann_path = Path(export).with_name(Path(export).name.split("_p")[0] + "_annotations.json")
    ann = json.loads(ann_path.read_text()) if ann_path.exists() else {}

    def produced(d):
        """Empire science/culture of state d: computed per city, times the recorded % modifier (amenities)."""
        routes = [(r["origin"], r["destination"]) for r in ann.get("trade_routes", []) if r["from_turn"] <= d["turn"]]
        by_name = {c["name"]: c for c in d["cities"]}
        total = 0.0
        for c in d["cities"]:
            v = city_yields(rules, c, d["player"], routes=routes, cities_by_name=by_name)[0][yield_key]
            rec = d["cityYields"][c["slot"]]
            b, m = rec["base"][yield_key], rec["modified"][yield_key]
            total += v * (m / b if b else 1)
        return total

    base_cost = dict(rules.q(f"select {col}, Cost from {table}"))
    era = dict(rules.q(f"select {col}, EraType from {table}"))
    pending = None  # (turn, item, total) of a completion awaiting its overflow
    for a, b in zip(rows, rows[1:]):
        pa, pb = a["player"], b["player"]
        y = produced(a)
        completed = [t for t in pb[done_key] if t not in pa[done_key]]
        boosted = [t for t in pb[boost_key] if t not in pa[boost_key]]
        grew = {t: pb[prog_key].get(t, 0) - pa[prog_key].get(t, 0) for t in set(pa[prog_key]) | set(pb[prog_key])}
        grew = {t: v for t, v in grew.items() if abs(v) > 1e-6 and t not in completed}
        line = f"T{a['turn']:2d} {yield_key[6:9]} {y:6.2f} |"
        for t in completed:
            p0 = pa[prog_key].get(t, 0)
            line += f" DONE {t.split('_', 1)[1]} ({p0:g}+{y:g}={p0 + y:g}; base {base_cost[t]}, {era[t][4:]})"
            pending = (a["turn"], t, p0 + y)
        for t, v in grew.items():
            extra = v - y
            tag = "boost" if t in boosted else ""
            if pending and abs(extra) > 0.05 and t not in boosted:
                pt, item, total = pending
                line += f" {t.split('_', 1)[1]} +{v:g} (overflow {extra:g} -> {item.split('_', 1)[1]} cost {total - extra:g})"
                pending = None
            elif abs(extra) > 0.05:
                line += f" {t.split('_', 1)[1]} +{v:g} ({tag or 'unexplained'} {extra:+g}; {extra / max(base_cost[t], 1) * 100:.0f}% of base {base_cost[t]})"
            else:
                line += f" {t.split('_', 1)[1]} +{v:g} ok"
        for t in boosted:
            if t not in grew and t not in completed:
                line += f" (boost {t.split('_', 1)[1]} recorded, progress unchanged)"
        print(line)


if __name__ == "__main__":
    main()
