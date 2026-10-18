"""List the modifier effects the engine resolved for one city (subject = the city or its districts/plots) and
for the player, marking those no consumer reads yet. Usage: python3 python_legacy/scripts/city_effects.py <rules> <export> <turn> <city>"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.modifiers import Engine  # noqa: E402
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.envs import player_of  # noqa: E402

CONSUMED = {
    "EFFECT_ADJUST_CITY_YIELD_CHANGE", "EFFECT_ADJUST_CITY_ALL_YIELDS_CHANGE", "EFFECT_ADJUST_BUILDING_YIELD_CHANGE",
    "EFFECT_ADJUST_CITY_YIELD_PER_POPULATION", "EFFECT_ADJUST_CITY_YIELD_PER_DISTRICT", "EFFECT_ADJUST_DISTRICT_YIELD_CHANGE",
    "EFFECT_ADJUST_DISTRICT_BASE_YIELD_CHANGE", "EFFECT_ADJUST_CITY_YIELD_MODIFIER", "EFFECT_FEATURE_ADJACENCY",
    "EFFECT_TERRAIN_ADJACENCY", "EFFECT_IMPROVEMENT_ADJACENCY", "EFFECT_ADJUST_TRADE_ROUTE_YIELD",
    "EFFECT_ADJUST_TRADE_ROUTE_YIELD_FOR_DOMESTIC", "EFFECT_ADJUST_TRADE_ROUTE_YIELD_FOR_INTERNATIONAL",
    "EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_FOR_DOMESTIC", "EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_FOR_INTERNATIONAL",
    "EFFECT_ADJUST_TRADE_ROUTE_YIELD_PER_SPECIALTY_DISTRICT_FOR_DOMESTIC", "EFFECT_ADJUST_PLAYER_TRADE_ROUTE_YIELD_PER_POST_IN_OWN_CITY",
    "EFFECT_ADJUST_CITY_STATE_TRADE_ROUTE_FLAT_YIELD",
}


def main():
    r = Rules(sys.argv[1])
    d = next(x for x in map(json.loads, open(sys.argv[2])) if x["turn"] == int(sys.argv[3]))
    e = Engine(r, player_of(d), d["cities"])
    name = sys.argv[4]
    for eff, rows in sorted(e.effects.items()):
        for subj, args, mid, src in rows:
            if subj == ("player",) or subj[1] == name:
                mark = "  " if eff in CONSUMED else "??"
                print(mark, eff[7:], subj[0], src, mid, {k: v for k, v in args.items() if k != "ModifierId"})
    print("plot effects:", [(m[4], m[0], m[1]) for m in e.plot_effects.get(name, ())])


if __name__ == "__main__":
    main()
