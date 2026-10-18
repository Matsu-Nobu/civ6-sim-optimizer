"""Everything the Rust simulator / search (optimizer) needs, in one JSON file.

  rules    every table of the rules database: columns and rows in rowid order, plus for every column the row
           numbers matching each value in the order SQLite returns them for "select ... where col=?" (an index
           may order them differently from rowid; the Rust side then sees rows in the same order as Python)
  map      the map (with river edges and roads)
  rows     the recorded game's export rows of the player (scenario: boosts, envoys, eras, wonders, ...)
  start    the planner's start state (FullPlanner.initial_state)
  settings the planner settings, the recorded game as a genome, establish turns of the governors

Usage: python3 python_legacy/scripts/export_problem.py <export prefix> <rules.sqlite> <out.json> [--start 2] [--horizon 60] [--player 0]
         [--no-genome] [--builder-candidates 4]   (skip replaying the recorded game in Python: enough for civ6opt validate / replay)
  e.g. python3 python_legacy/scripts/export_problem.py data/exports/game_20261005 data/rules/gameplay_2026-10-05.sqlite \
       data/games/rome_20261005.json --horizon 60
"""
import json
import sqlite3
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from civ6sim.plan2 import FullPlanner, Scenario, Settings, recorded_choices, run_recorded  # noqa: E402
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.world import Map, World  # noqa: E402


def arg(name, default, cast=int):
    return cast(sys.argv[sys.argv.index(name) + 1]) if name in sys.argv else default


def jsonable(v):
    """Sets -> sorted lists, tuples -> lists, dataclasses -> dicts (for the start state)."""
    if hasattr(v, "__dataclass_fields__"):
        return {k: jsonable(getattr(v, k)) for k in v.__dataclass_fields__}
    if isinstance(v, dict):
        return {(k if isinstance(k, str) else json.dumps(jsonable(k))): jsonable(x) for k, x in v.items()}
    if isinstance(v, (set, frozenset)):
        return sorted(jsonable(x) for x in v)
    if isinstance(v, (list, tuple)):
        return [jsonable(x) for x in v]
    return v


def export_rules(db_path):
    db = sqlite3.connect(db_path)
    out = {}
    for (t,) in db.execute("select name from sqlite_master where type='table' and name not like 'sqlite_%'"):
        cols = [c[1] for c in db.execute(f'pragma table_info("{t}")')]
        # rowid and row from the same query: "select rowid" alone may come back in an index's order
        both = [list(r) for r in db.execute(f'select rowid, * from "{t}" order by rowid')]
        ids = [r[0] for r in both]
        rows = [r[1:] for r in both]
        pos = {rid: i for i, rid in enumerate(ids)}
        order = {}
        for c in cols:
            vals = {r[0] for r in db.execute(f'select distinct "{c}" from "{t}"')}
            if len(vals) == len(rows) and len(rows) > 64:
                continue  # unique column: lookups find one row; no order to keep
            by = {}
            for v in vals:
                if v is None:
                    continue
                by[str(v)] = [pos[r[0]] for r in db.execute(f'select rowid from "{t}" where "{c}"=?', (v,))]
            order[c] = by
        out[t] = {"cols": cols, "rows": rows, "order": order}
    # whole-table reads of the Python code, in the order SQLite returned them (a one-column select may come back in an
    # index's order): name -> row numbers of the table, mapped back by the first selected column (a unique key)
    scans = {}
    for name, (sql, table, key) in SCANS.items():
        t = out.get(table)
        if t is None:
            continue
        kc = t["cols"].index(key)
        pos = {}
        for i, r in enumerate(t["rows"]):
            pos.setdefault(r[kc], []).append(i)
        seen = {}
        idx = []
        for (v, *_rest) in db.execute(sql):
            k = seen.get(v, 0)
            seen[v] = k + 1
            idx.append(pos[v][k])
        scans[name] = idx
    out["__scans__"] = scans
    return out


SCANS = {
    "Technologies": ("select TechnologyType from Technologies", "Technologies", "TechnologyType"),
    "Civics": ("select CivicType from Civics", "Civics", "CivicType"),
    "GameModifiers": ("select ModifierId from GameModifiers", "GameModifiers", "ModifierId"),
    "Pantheons": ("select BeliefType from Beliefs where BeliefClassType='BELIEF_CLASS_PANTHEON'", "Beliefs", "BeliefType"),
    "Governments": ("select GovernmentType, PrereqCivic from Governments", "Governments", "GovernmentType"),
    "Policies": ("select PolicyType, PrereqCivic, PrereqTech, GovernmentSlotType from Policies where PolicyType not like 'POLICY_GOV_%'",
                 "Policies", "PolicyType"),
    "Units": ("select UnitType, PrereqTech, PrereqCivic, TraitType, Domain, CanTrain, PromotionClass, "
              "ReligiousStrength > 0 or SpreadCharges > 0 or FoundReligion or EnabledByReligion from Units", "Units", "UnitType"),
    "Buildings": ("select BuildingType, PrereqTech, PrereqCivic, PrereqDistrict, TraitType, IsWonder from Buildings where InternalOnly=0",
                  "Buildings", "BuildingType"),
    "Districts": ("select DistrictType, PrereqTech, PrereqCivic, TraitType, RequiresPopulation, OnePerCity, MaxPerPlayer "
                  "from Districts where InternalOnly=0 and CityCenter=0", "Districts", "DistrictType"),
    "Projects": ("select ProjectType, PrereqTech, PrereqCivic, PrereqDistrict, MaxPlayerInstances, SpaceRace, WMD, UnlocksFromEffect "
                 "from Projects", "Projects", "ProjectType"),
    "Improvements": ("select ImprovementType, PrereqTech, PrereqCivic, TraitType, SameAdjacentValid, RequiresRiver, MinimumAppeal, Coast, "
                     "ValidAdjacentTerrainAmount, RequiresAdjacentBonusOrLuxury, RequiresAdjacentLuxury, NoAdjacentSpecialtyDistrict, "
                     "AdjacentToLand, OnePerCity, Domain from Improvements where Buildable=1", "Improvements", "ImprovementType"),
    "Governors": ("select GovernorType, AssignCityState, TraitType from Governors", "Governors", "GovernorType"),
    "PlantableFeatures": ("select FeatureType, AddCivic from Features where AddCivic is not null", "Features", "FeatureType"),
}


def main():
    prefix, db, out = sys.argv[1], sys.argv[2], sys.argv[3]
    start, horizon, player = arg("--start", 2), arg("--horizon", 60), arg("--player", 0)
    no_genome = "--no-genome" in sys.argv
    rules = Rules(db)
    rows = [json.loads(line) for line in open(f"{prefix}_p{player}.jsonl")]
    game_map = json.load(open(f"{prefix}_map_T{start}.json"))
    planner = FullPlanner(rules, World(rules, Map(game_map)), Scenario(rows, rules), Settings(horizon=horizon, builder_candidates=arg("--builder-candidates", 4)))
    st0 = planner.initial_state(start)
    rec = recorded_choices(rows)
    if no_genome:
        rec_st = None
        genome = []
    else:
        rec_st, _ = run_recorded(planner, st0, rows)
        genome = [[list(k), jsonable(c)] for k, _, c in rec_st.log if c is not None]
    genome += [[["settle", "", n], list(xy)] for n, (_, xy) in enumerate(rec["sites"])]
    # the same with the recorded cities' tiles (Settings.recorded_tiles; civ6opt run --genome recorded_tiles)
    genome_t = []
    if not no_genome:
        rec_st_t, _ = run_recorded(planner, st0, rows, tiles=True)
        genome_t = [[list(k), jsonable(c)] for k, _, c in rec_st_t.log if c is not None]
    genome_t += [[["settle", "", n], list(xy)] for n, (_, xy) in enumerate(rec["sites"])]
    sc = planner.sc
    problem = {
        "rules": export_rules(db),
        "map": game_map,
        "rows": rows,
        "start": jsonable(st0),
        "settings": {
            "horizon": horizon, "max_cities": planner.s.max_cities, "settle_candidates": planner.s.settle_candidates,
            "settle_radius": planner.s.settle_radius, "builder_candidates": planner.s.builder_candidates,
            "fixed": [[list(k), v] for k, v in planner.s.fixed.items()],
            "extra_sites": [list(xy) for _, xy in rec["sites"]],
            "site_names": [[list(xy), nm] for nm, xy in rec["sites"]],
            "extra_builder": [jsonable(c) for k, _, c in rec_st.log if k[0] == "builder" and c and c[0] != "wait"] if rec_st else [],
            "speed": planner.speed,
            "recorded_tiles": [[list(cxy), [[t, list(xy)] for t, xy in v]] for cxy, v in rec["tiles"].items()],
        },
        "recorded_genome": genome,
        "recorded_genome_tiles": genome_t,
        "establish_turns": {g: planner.establish_turns(g) for (g,) in rules.q("select GovernorType from Governors")},
        # scenario pieces computed in Python (exact same as the planner's)
        "scenario": {
            "exo_boosts": {str(t): v for t, v in sc.exo_boosts.items()},
            "faith_gift": {str(t): v for t, v in sc.faith_gift.items()},
            "gold_gift": {str(t): v for t, v in sc.gold_gift.items()},
            "scouts_by_turn": {str(t): v for t, v in sc.scouts_by_turn.items()},
            "unit_gifts": {str(t): v for t, v in sc.unit_gifts.items()},
            "gp_timeline": {cls: [[i, f, tk] for i, f, tk in v] for cls, v in sc.gp_timeline().items()},
        },
    }
    Path(out).parent.mkdir(parents=True, exist_ok=True)
    Path(out).write_text(json.dumps(problem))
    print(f"wrote {out}: {Path(out).stat().st_size / 1e6:.1f} MB")


if __name__ == "__main__":
    main()
