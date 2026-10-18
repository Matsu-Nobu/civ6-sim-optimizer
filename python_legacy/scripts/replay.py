"""Free-running replay: start from the first save with a city and let the simulator carry production,
food, population, research and gold, taking the player's choices (items, research, tiles, policies,
boosts) from the saves. Prints where the simulated accumulators drift from the saves.

Usage: python3 python_legacy/scripts/replay.py <export.jsonl> <rules.sqlite> [--verbose]
"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.sim import PlayerAcc, step  # noqa: E402
from civ6sim.envs import player_of  # noqa: E402


def grew(a, b, boosted):
    """What was being researched: the progress entry that grew, ignoring growth that is only a boost."""
    best, best_d = None, 0.0
    for t in set(a) | set(b):
        dlt = b.get(t, 0) - a.get(t, 0)
        if t in boosted:
            dlt -= 1e6  # prefer non-boosted entries; a boosted one only if nothing else grew
        if dlt > best_d or (best is None and dlt > -1e6 + 1e-6):
            best, best_d = t, dlt
    if best is not None and best_d <= 0:
        # Nothing but boosted entries grew: take the boosted one that grew most (it was also being researched).
        cands = [(b.get(t, 0) - a.get(t, 0), t) for t in boosted if b.get(t, 0) - a.get(t, 0) > 0]
        return max(cands)[1] if cands else None
    return best


def main():
    export, db = sys.argv[1], sys.argv[2]
    verbose = "--verbose" in sys.argv
    rules = Rules(db)
    rows = [json.loads(line) for line in open(export)]
    ann_path = Path(export).with_name(Path(export).name.split("_p")[0] + "_annotations.json")
    ann = json.loads(ann_path.read_text()) if ann_path.exists() else {}
    rows = [d for d in rows if d["cities"]]
    first = rows[0]
    p = first["player"]
    acc = PlayerAcc(gold=p["gold"], tech_progress=dict(p["techProgress"]), civic_progress=dict(p["civicProgress"]),
                    techs=set(p["techs"]), civics=set(p["civics"]), units_trained=dict(p["unitsTrained"]))
    gold_purchases = 0.0
    worst = {}
    for a, b in zip(rows, rows[1:]):
        pa, pb = a["player"], b["player"]
        boosts = [t for t in pb["techsBoosted"] + pb["civicsInspired"] if t not in pa["techsBoosted"] + pa["civicsInspired"]]
        # Ignore boost flags without a Boosts row (no progress), so they don't hide the item being researched.
        real_boosts = [t for t in boosts if rules.has_boost(t)]
        # A newly finished item is the one being researched unless a boost finished it.
        # The item being researched: the save's current one when it finished this turn (a boost and the turn's
        # science can finish it together), else a newly finished item no boost explains, else the one that grew.
        cur_t, cur_c = pa.get("currentResearch"), pa.get("currentCivic")
        research = (cur_t if cur_t in pb["techs"] and cur_t not in pa["techs"] else None) \
            or next((t for t in pb["techs"] if t not in pa["techs"] and t not in real_boosts), None) \
            or grew(pa["techProgress"], pb["techProgress"], real_boosts)
        civic = (cur_c if cur_c in pb["civics"] and cur_c not in pa["civics"] else None) \
            or next((t for t in pb["civics"] if t not in pa["civics"] and t not in real_boosts), None) \
            or grew(pa["civicProgress"], pb["civicProgress"], real_boosts)
        acc.units_trained = dict(pa["unitsTrained"])  # purchases are outside the simulator: take the count from the save
        routes = [(r["origin"], r["destination"]) for r in ann.get("trade_routes", []) if r["from_turn"] <= a["turn"]]
        nxt = {c["name"]: c for c in b["cities"]}
        # research completes against the costs of the coming turn's game era (an era switch happens first)
        a = dict(a, player=player_of(a, b.get("gameEra")))
        log = step(rules, acc, a, a["gameSpeed"], research=research, civic=civic, boosts=boosts, routes=routes,
                   after_settler=lambda name: nxt.get(name))
        # Tribal-village gold (exploration, outside the model): take the save's gold when a hut was entered.
        huts = lambda p: sum((p.get("goodyHutsReceived") or {}).values())
        if huts(pb) > huts(pa) and pb["gold"] > acc.gold:
            acc.gold = pb["gold"]
        # Purchases are player actions not in the save: take the unexplained gold drop as spent.
        spent = acc.gold - pb["gold"]
        if spent > 1:
            gold_purchases += spent
            acc.gold = pb["gold"]
        diffs = []
        for c in b["cities"]:
            ca = acc.cities.get(c["name"])
            if not ca:
                continue
            if ca.population != c["population"]:
                diffs.append(f"{c['name']} pop {ca.population} vs {c['population']}")
            if c["foodStore"] is not None and abs(ca.food_store - c["foodStore"]) > 0.05:
                diffs.append(f"{c['name']} food {ca.food_store:.2f} vs {c['foodStore']:.2f}")
            item = c["production"]["current"]
            if item and abs(ca.progress.get(item, 0) - c["production"]["progress"].get(item, 0)) > 0.05:
                diffs.append(f"{c['name']} {item} {ca.progress.get(item, 0):.2f} vs {c['production']['progress'].get(item, 0):.2f}")
        for kind, prog, real in (("tech", acc.tech_progress, pb["techProgress"]), ("civic", acc.civic_progress, pb["civicProgress"])):
            for t, v in real.items():
                if abs(prog.get(t, 0) - v) > 0.05:
                    diffs.append(f"{t} {prog.get(t, 0):.2f} vs {v:.2f}")
        if abs(acc.gold - pb["gold"]) > 0.05:
            diffs.append(f"gold {acc.gold:.2f} vs {pb['gold']:.2f}")
        events = ", ".join(f"{e[0]} {e[-1]}" for e in log)
        print(f"T{b['turn']:2d} {'OK ' if not diffs else 'DIFF'} {events if verbose or diffs else ''}")
        for x in diffs:
            print("        ", x)
    print(f"gold spent on purchases (inferred): {gold_purchases:.0f}")


if __name__ == "__main__":
    main()
