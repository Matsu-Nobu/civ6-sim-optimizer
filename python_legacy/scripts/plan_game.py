"""Run the production planner from a recorded game's turn and compare with what was actually played.

Usage: python3 python_legacy/scripts/plan_game.py <game prefix> <rules.sqlite> [--start 2] [--horizon 31] [--passes 1]
  e.g. python3 python_legacy/scripts/plan_game.py data/exports/game_20261005 data/rules/gameplay_2026-10-05.sqlite
"""
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.plan import Planner, Scenario, Settings, recorded_sites, run_recorded  # noqa: E402
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.world import Map, World  # noqa: E402


def arg(name, default, cast=int):
    return cast(sys.argv[sys.argv.index(name) + 1]) if name in sys.argv else default


def main():
    prefix, db = sys.argv[1], sys.argv[2]
    start, horizon, passes = arg("--start", 2), arg("--horizon", 31), arg("--passes", 1)
    rules = Rules(db)
    rows = [json.loads(line) for line in open(f"{prefix}_p0.jsonl")]
    ann_path = Path(f"{prefix}_annotations.json")
    ann = json.loads(ann_path.read_text()) if ann_path.exists() else {}
    game_map = Map(json.load(open(sorted(Path(prefix).parent.glob(Path(prefix).name + "_map_T*.json"))[0])))
    sites = recorded_sites(rows) if "--same-sites" in sys.argv else None
    planner = Planner(rules, World(rules, game_map), Scenario(rows, ann), Settings(horizon=horizon, sites=sites))
    st0 = planner.initial_state(start)
    # The recorded choices, evaluated in the same model (settling where the player actually settled).
    rec_planner = Planner(rules, World(rules, game_map), Scenario(rows, ann), Settings(horizon=horizon, sites=recorded_sites(rows)))
    rec_final, rec_value = run_recorded(rec_planner, rec_planner.initial_state(start), rows)
    print(f"recorded choices in the model: value {rec_value:.1f}")
    for c in rec_final.cities:
        print(f"  {c['name']:8s} pop {rec_final.acc.cities[c['name']].population} buildings {[b[9:] for b in c['buildings']]} "
              f"districts {[x['type'][9:] for x in c['districts'] if x['complete'] and x['type'] != 'DISTRICT_CITY_CENTER']}")
    print("  per-turn output at the end:", {k[6:]: round(v, 1) for k, v in rec_final.last_output.items()})
    t0 = time.time()
    plan, final, value = planner.rollout_search(st0, passes=passes)
    print(f"search took {time.time() - t0:.1f}s")
    print(f"\nplan value {value:.1f}; state at T{final.turn - 1}:")
    for c in final.cities:
        print(f"  {c['name']:8s} pop {final.acc.cities[c['name']].population} at {c['center']['x']},{c['center']['y']} "
              f"buildings {[b[9:] for b in c['buildings']]} districts {[x['type'][9:] for x in c['districts'] if x['complete'] and x['type'] != 'DISTRICT_CITY_CENTER']}")
    print("  per-turn output at the end:", {k[6:]: round(v, 1) for k, v in final.last_output.items()})
    print("\ndecisions:")
    for t, city, item in final.decisions:
        print(f"  T{t:2d} {city:8s} {item}")
    # The recorded game, for comparison.
    last = rows[-1]
    print(f"\nrecorded game at T{last['turn']}:")
    for c in last["cities"]:
        print(f"  {c['name']:8s} pop {c['population']} at {c['center']['x']},{c['center']['y']} buildings {[b[9:] for b in c['buildings']]}")
    print("  per-turn output (recorded):", {k[6:]: round(v, 1) for k, v in last["player"]["yields"].items()})


if __name__ == "__main__":
    main()
