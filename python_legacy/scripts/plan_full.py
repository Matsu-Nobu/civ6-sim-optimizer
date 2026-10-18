"""Full-action planner: every economic decision searched. Compares with the recorded game in the same model.

Usage: python3 python_legacy/scripts/plan_full.py <game prefix> <rules.sqlite> [--start 2] [--horizon 31] [--passes 2]
"""
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.plan2 import FullPlanner, Scenario, Settings, recorded_choices, recorded_factory, run_recorded  # noqa: E402
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.world import Map, World  # noqa: E402


def arg(name, default, cast=int):
    return cast(sys.argv[sys.argv.index(name) + 1]) if name in sys.argv else default


def summary(planner, st, value, title):
    print(f"\n== {title}: value {value:.1f}")
    for c in st.cities:
        ca = st.acc.cities[c["name"]]
        print(f"  {c['name']:8s} pop {ca.population} at {c['center']['x']},{c['center']['y']} buildings {[b[9:] for b in c['buildings']]} "
              f"districts {[x['type'][9:] for x in c['districts'] if x['complete'] and x['type'] != 'DISTRICT_CITY_CENTER']}")
    print("  output/turn:", {k[6:]: round(v, 1) for k, v in st.last_output.items()})
    print(f"  techs {len(st.acc.techs)} civics {len(st.acc.civics)} gov {st.player['government'][11:]} policies {[p[7:] for p in st.player['policies']]} pantheon {st.player.get('pantheon')}")


def main():
    prefix, db = sys.argv[1], sys.argv[2]
    start, horizon, passes = arg("--start", 2), arg("--horizon", 31), arg("--passes", 2)
    rules = Rules(db)
    rows = [json.loads(line) for line in open(f"{prefix}_p0.jsonl")]
    game_map = Map(json.load(open(sorted(Path(prefix).parent.glob(Path(prefix).name + "_map_T*.json"))[0])))
    planner = FullPlanner(rules, World(rules, game_map), Scenario(rows, rules), Settings(horizon=horizon))
    st0 = planner.initial_state(start)
    rec_final, rec_value = run_recorded(planner, st0, rows)
    summary(planner, rec_final, rec_value, "recorded game (your choices) in this model")
    t0 = time.time()
    final, value = planner.run(st0, {})
    print(f"one rollout: {time.time() - t0:.2f}s, {len(final.log)} decisions")
    summary(planner, final, value, "base policy")
    t0 = time.time()
    plan, final, value = planner.search(st0, passes=passes)
    print(f"search from the base policy took {time.time() - t0:.0f}s")
    summary(planner, final, value, "searched plan (from the base policy)")
    # Search starting from the recorded game: improvements to the player's own decisions.
    rec = recorded_choices(rows)
    planner.s.sites_saved = planner.s.sites
    t0 = time.time()
    plan_r, final_r, value_r = planner.search(st0, passes=passes, base_factory=recorded_factory(planner, rows))
    print(f"search from the recorded game took {time.time() - t0:.0f}s")
    summary(planner, final_r, value_r, "improved recorded game")
    print(f"\nrecorded {rec_value:.1f} -> improved {value_r:.1f} ({(value_r / rec_value - 1) * 100:+.1f}%), "
          f"from scratch {value:.1f} ({(value / rec_value - 1) * 100:+.1f}%)")
    print("\nimprovements to the recorded decisions (in the order found; gain = value increase at that step):")
    for key, turn, old, new, gain in planner.improvements:
        print(f"  T{turn if turn is not None else '?':>2} {key[0]:8s} {str(key[1]):8s} #{key[2]:<2d} {str(old)[:40]:40s} -> {str(new)[:40]:40s} +{gain:.1f}")


if __name__ == "__main__":
    main()
