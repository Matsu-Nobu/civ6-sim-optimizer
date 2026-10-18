"""Check the simulator's numbers against the game's own (Econ Probe mod output), turn by turn.

Usage: python3 python_legacy/scripts/check_probe.py <export.jsonl> <rules.sqlite> <econprobe.jsonl>
Reports, per category, how many values match and lists the mismatches.
"""
import json
import math
import sys
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.amenities import amenity_scores, happiness  # noqa: E402
from civ6sim.growth import growth_threshold, housing  # noqa: E402
from civ6sim.production import item_cost  # noqa: E402
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.sim import research_cost  # noqa: E402
from civ6sim.yields import city_yields, tile_yields  # noqa: E402
from civ6sim.envs import player_of  # noqa: E402

SHORT = {"YIELD_FOOD": "F", "YIELD_PRODUCTION": "P", "YIELD_GOLD": "G", "YIELD_SCIENCE": "S", "YIELD_CULTURE": "C", "YIELD_FAITH": "R"}


def main():
    export, db, probe = sys.argv[1], sys.argv[2], sys.argv[3]
    rules = Rules(db)
    rows = {d["turn"]: d for d in (json.loads(l) for l in open(export))}
    probes = {}
    for line in open(probe):
        p = json.loads(line)
        probes[p["turn"]] = p  # last dump of a turn wins
    stats = defaultdict(lambda: [0, 0])
    bad = defaultdict(list)

    def check(cat, ok, msg):
        stats[cat][0 if ok else 1] += 1
        if not ok:
            bad[cat].append(msg)

    args = sys.argv[4:]
    offset = int(args[args.index("--offset") + 1]) if "--offset" in args else 0  # compare probe T with save T+offset
    skip = set(int(x) for x in args[0].split(",")) if args and not args[0].startswith("--") else set()
    eras = ("ERA_FUTURE", "ERA_INFORMATION")  # far-future costs are irrelevant here and skipped
    for turn, pr in sorted(probes.items()):
        if turn in skip:
            continue
        d = rows.get(turn + offset)
        if not d:
            continue
        player = player_of(d)
        speed = d["gameSpeed"]
        by_xy = {(c["center"]["x"], c["center"]["y"]): c for c in d["cities"]}
        scores = amenity_scores(rules, d["cities"], player_of(d))
        # research costs
        for t, cost, prog, has, boosted in pr["techs"] + pr["civics"]:
            if cost is None:
                continue
            tab, col = ("Technologies", "TechnologyType") if t.startswith("TECH_") else ("Civics", "CivicType")
            if rules.q(f"select EraType from {tab} where {col}=?", t)[0][0] in eras:
                continue
            ours = research_cost(rules, t, speed, d.get("gameEra") or "ERA_ANCIENT")
            check("research cost", abs(ours - cost) < 0.5, f"T{turn} {t}: game {cost}, ours {ours}")
        for pc in pr["cities"]:
            c = by_xy.get((pc["x"], pc["y"]))
            if not c:
                continue
            name = c["name"]
            check("growth threshold", growth_threshold(rules, c["population"], speed) == pc["growthThreshold"],
                  f"T{turn} {name} pop{c['population']}: game {pc['growthThreshold']}, ours {growth_threshold(rules, c['population'], speed)}")
            if c["foodStore"] is not None and pc.get("food") is not None:
                check("food store (save reader)", abs(c["foodStore"] - pc["food"]) < 0.01, f"T{turn} {name}: game {pc['food']}, save {c['foodStore']}")
            h = housing(rules, c, player, d["cities"])
            check("housing (shown rounded down)", math.floor(h) == pc["housing"], f"T{turn} {name}: game {pc['housing']}, ours {h}")
            need = math.ceil(c["population"] / rules.param("CITY_POP_PER_AMENITY"))
            from civ6sim.amenities import amenities_needed as _an
            need = _an(rules, c["population"])
            check("amenities needed", need == pc["amenitiesNeeded"], f"T{turn} {name}: game {pc['amenitiesNeeded']}, ours {need}")
            from civ6sim.amenities import amenities_needed
            need = amenities_needed(rules, c["population"])
            supply = scores[name] + need
            check("amenities", abs(supply - pc["amenities"]) < 0.01, f"T{turn} {name}: game {pc['amenities']}, ours {supply}")
            # city yields (after % modifiers)
            y, _ = city_yields(rules, c, player, routes=d.get("tradeRoutes", ()), cities_by_name={x["name"]: x for x in d["cities"]})
            from civ6sim.yields import percent_modifiers
            from civ6sim.modifiers import engine_for
            from civ6sim.yields import city_yield_percents
            pcts = percent_modifiers(happiness(rules, scores[name])[0], is_ai="--ai" in args,
                                     extra=city_yield_percents(rules, engine_for(rules, player, d["cities"]), c, scores[name]))
            for k, v in pc["yields"].items():
                ours = y.get(k, 0) * (1 + pcts[k] / 100)
                check(f"city yield {SHORT[k]}", abs(ours - v) < 0.06, f"T{turn} {name} {k}: game {v}, ours {ours:.3f}")
            # tile yields
            owned = {(p["x"], p["y"]): p for p in c["owned"]}
            techs, civics = set(player["techs"]), set(player["civics"])
            for x, yy, ty, workers in pc["tiles"]:
                p = owned.get((x, yy))
                if not p:
                    check("tile ownership", False, f"T{turn} {name} ({x},{yy}) owned in game, not in save export")
                    continue
                if p.get("district") and (x, yy) != (pc["x"], pc["y"]):
                    continue  # district plots: the game shows the district's adjacency there, checked as city yields
                from civ6sim.yields import city_plot_modifiers, plots_by_xy_of
                mods = city_plot_modifiers(rules, player, c, d["cities"])
                all_plots = plots_by_xy_of(d["cities"])
                p = all_plots.get((x, yy), p)
                ours = tile_yields(rules, p, techs, civics, city_center=(x, yy) == (pc["x"], pc["y"]),
                                   bbcc=c["capital"] and (x, yy) == (pc["x"], pc["y"]), plot_modifiers=mods, plots_by_xy=all_plots, city=c)
                diff = {SHORT[k]: v - ours.get(k, 0) for k, v in ty.items() if abs(v - ours.get(k, 0)) > 0.01}
                desc = "/".join(s.split("_", 1)[1] for s in (p["terrain"], p["feature"], p["resource"], p["improvement"]) if s)
                check("tile yield", not diff, f"T{turn} {name} ({x},{yy}) {desc}: game-ours {diff}")
            # border growth cost: n = tiles gained by culture, unknown from a save (purchases), so only the formula
            # is checked: some n >= 0 must give the game's cost.
            n = next((k for k in range(0, 40) if math.floor(0.5 * (rules.param("CULTURE_COST_FIRST_PLOT") + (rules.param("CULTURE_COST_LATER_PLOT_MULTIPLIER") * k) ** rules.param("CULTURE_COST_LATER_PLOT_EXPONENT"))) == pc["nextPlotCultureCost"]), len(c["owned"]) - 7)
            ours_b = math.floor(0.5 * (rules.param("CULTURE_COST_FIRST_PLOT") + (rules.param("CULTURE_COST_LATER_PLOT_MULTIPLIER") * n)
                                     ** rules.param("CULTURE_COST_LATER_PLOT_EXPONENT")))
            check("border cost formula (some n fits)", ours_b == pc["nextPlotCultureCost"],
                  f"T{turn} {name} tiles {len(c['owned'])}: game {pc['nextPlotCultureCost']}, ours {ours_b}")
            # items: production cost and purchase price
            for it in pc["items"]:
                if it["cost"] is None:
                    continue
                from collections import Counter as _C
                spec = {x for (x,) in rules.q("select DistrictType from Districts where RequiresPopulation=1")}
                placed = _C(x["type"] for cc in d["cities"] for x in cc["districts"])
                done = sum(1 for cc in d["cities"] for x in cc["districts"] if x["complete"] and x["type"] in spec)
                pl_cost = dict(player, _districts_completed=done)
                if it["type"].startswith("DISTRICT_") and any(x["type"] == it["type"] and not x["complete"] for x in c["districts"]):
                    continue  # placed here: its cost was locked when placed
                ours = item_cost(rules, pl_cost, it["type"], speed, placed_counts=placed)
                if ours is not None:
                    check(f"cost {it['type'].split('_')[0].lower()}", abs(ours - it["cost"]) < 0.5, f"T{turn} {name} {it['type']}: game {it['cost']}, ours {ours}")
                if it.get("gold"):
                    # districts are priced on the rounded cost, buildings and units on the unrounded one
                    exact = item_cost(rules, pl_cost, it["type"], speed, placed_counts=placed,
                                      exact=not it["type"].startswith("DISTRICT_"))
                    from civ6sim.production import gold_price
                    from civ6sim.modifiers import engine_for
                    from civ6sim.production import purchase_discount_percent
                    disc = purchase_discount_percent(rules, engine_for(rules, player, d["cities"]), c, it["type"])
                    guess = gold_price(exact if exact is not None else it["cost"], disc)
                    check("gold price", guess == it["gold"], f"T{turn} {name} {it['type']}: game {it['gold']} (cost {it['cost']}), formula {guess}")
            for x, yy, price in pc["buyableTiles"]:
                pass
    for cat, (ok, ng) in sorted(stats.items()):
        print(f"{cat:34s} {ok:4d} match {ng:4d} differ")
    for cat, msgs in bad.items():
        print(f"\n[{cat}]")
        for m in msgs[:8]:
            print("  ", m)
        if len(msgs) > 8:
            print(f"   ... {len(msgs) - 8} more")


if __name__ == "__main__":
    main()
