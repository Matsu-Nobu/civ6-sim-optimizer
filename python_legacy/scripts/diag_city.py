"""Per-city diagnosis at one turn: game vs our yields, our breakdown, and tile-level differences.

Usage: python3 python_legacy/scripts/diag_city.py <export.jsonl> <rules.sqlite> <probe.jsonl> <probe turn> [--offset -1]
"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.amenities import amenity_scores, happiness  # noqa: E402
from civ6sim.growth import housing  # noqa: E402
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.yields import city_yields, city_plot_modifiers, plots_by_xy_of, tile_yields  # noqa: E402
from civ6sim.envs import player_of  # noqa: E402

SH = {"YIELD_FOOD": "F", "YIELD_PRODUCTION": "P", "YIELD_GOLD": "G", "YIELD_SCIENCE": "S", "YIELD_CULTURE": "C", "YIELD_FAITH": "R"}


def main():
    export, db, probe, turn = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
    off = int(sys.argv[sys.argv.index("--offset") + 1]) if "--offset" in sys.argv else 0
    r = Rules(db)
    rows = {d["turn"]: d for d in (json.loads(l) for l in open(export))}
    pr = next(json.loads(l) for l in open(probe) if json.loads(l)["turn"] == turn)
    d = rows[turn + off]
    player = player_of(d)
    print(f"probe T{turn} vs save T{turn + off}: gov {player['government']} policies {player['policies']} pantheon {player.get('pantheon')} "
          f"era {d.get('gameEra')} techs {len(player['techs'])} civics {len(player['civics'])}")
    scores = amenity_scores(r, d["cities"])
    all_plots = plots_by_xy_of(d["cities"])
    by_name = {c["name"]: c for c in d["cities"]}
    for pc in pr["cities"]:
        c = next((x for x in d["cities"] if (x["center"]["x"], x["center"]["y"]) == (pc["x"], pc["y"])), None)
        if not c:
            continue
        y, parts = city_yields(r, c, player, cities_by_name=by_name)
        pct = happiness(r, scores[c["name"]])[0]
        from civ6sim.yields import percent_modifiers
        pcts = percent_modifiers(pct, is_ai="--ai" in sys.argv)
        print(f"\n== {c['name']} pop {c['population']} amen game {pc['amenities']}-{pc['amenitiesNeeded']} ours score {scores[c['name']]} ({pct:+g}%) "
              f"housing game {pc['housing']} ours {housing(r, c)}")
        print("   game:", {SH[k]: round(v, 2) for k, v in pc["yields"].items()})
        print("   ours:", {SH[k]: round(y[k] * (1 + pcts[k] / 100), 2) for k in SH})
        print("   buildings:", [b[9:] for b in c["buildings"]], "districts:", [x["type"][9:] + ("" if x["complete"] else "*") for x in c["districts"]])
        for k, v in parts.items():
            if not k.startswith("tile(") and k != "center":
                print("     ", k, {SH[kk]: vv for kk, vv in v.items() if vv})
        mods = city_plot_modifiers(r, player, c, d["cities"])
        owned = {(p["x"], p["y"]): p for p in c["owned"]}
        worked = {(w["x"], w["y"]) for w in c["worked"]}
        for x, yy, ty, workers in pc["tiles"]:
            p = owned.get((x, yy))
            if not p:
                continue
            is_c = (x, yy) == (pc["x"], pc["y"])
            ours = tile_yields(r, p, set(player["techs"]), set(player["civics"]), city_center=is_c, bbcc=c["capital"] and is_c,
                               plot_modifiers=mods, plots_by_xy=all_plots, city=c)
            diff = {SH[k]: round(v - ours.get(k, 0), 2) for k, v in ty.items() if abs(v - ours.get(k, 0)) > 0.01}
            if diff or ((x, yy) in worked) != bool(workers):
                desc = "/".join(s.split("_", 1)[1] for s in (p["terrain"], p["feature"], p["resource"], p["improvement"]) if s)
                print(f"     tile ({x},{yy}) {desc:38s} worked game {workers} save {int((x, yy) in worked)} diff {diff}")


if __name__ == "__main__":
    main()
