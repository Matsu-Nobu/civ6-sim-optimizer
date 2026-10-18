"""Broad strategy search (MAP-Elites) on the recorded Rome start.

Usage: python3 python_legacy/scripts/search_broad.py [--budget 20000] [--workers 15] [--out data/search/broad.json]
"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.plan2 import FullPlanner, Scenario, Settings  # noqa: E402
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.search import map_elites  # noqa: E402
from civ6sim.world import Map, World  # noqa: E402

PREFIX, DB, HORIZON = "data/exports/game_20261005", "data/rules/gameplay_2026-10-05.sqlite", 60


def arg(name, default, cast=int):
    return cast(sys.argv[sys.argv.index(name) + 1]) if name in sys.argv else default


def build():
    rules = Rules(DB)
    rows = [json.loads(line) for line in open(f"{PREFIX}_p0.jsonl")]
    game_map = Map(json.load(open(f"{PREFIX}_map_T2.json")))
    planner = FullPlanner(rules, World(rules, game_map), Scenario(rows, rules), Settings(horizon=HORIZON))
    return planner, planner.initial_state(2), HORIZON


def main():
    out = arg("--out", "data/search/broad.json", str)
    Path(out).parent.mkdir(parents=True, exist_ok=True)
    map_elites(build, budget=arg("--budget", 20000), workers=arg("--workers", 15),
               init=arg("--init", 300), out=out)
    print("saved", out)


if __name__ == "__main__":
    main()
