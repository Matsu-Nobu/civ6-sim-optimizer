"""Which plot does a city's border take next? Test scoring rules against the game's own choice (probe "nextPlot")
in recorded games. The rule family is Civ5's (same GlobalParameters PLOT_INFLUENCE_*): the lowest influence cost
wins; cost = distance term + resource/natural wonder/water terms + yield points.

Usage: python3 python_legacy/scripts/border_fit.py [game ...]   (default: rome1 rome2 auto3 auto4)
"""
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.world import Map  # noqa: E402
from civ6sim.yields import tile_yields  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
CACHE = Path("/tmp/claude-1000/-home-nobu/0ca7bf42-0b40-42d6-b390-a953841d5c5b/scratchpad/maps")
RULES = {"rome1": "gameplay_rome1", "rome2": "gameplay_rome2", "auto3": "gameplay_rome1", "auto4": "gameplay_rome1"}


def map_at(game, turn):
    """The map from the save of `turn` (cached)."""
    CACHE.mkdir(parents=True, exist_ok=True)
    out = CACHE / f"{game}_T{turn:03d}.json"
    if not out.exists():
        save = next((p for p in sorted((ROOT / f"data/saves/game_{game}").glob(f"*_T{turn:03d}.Civ6Save"))), None)
        if save is None:
            return None
        node = "node"
        out.write_text(subprocess.run([str(node), str(ROOT / "save_export/export_map.js"), str(save)], capture_output=True,
                                      text=True, check=True).stdout)
    return json.loads(out.read_text())


def cases(game):
    """(turn, player, city xy, owned set, candidates [(xy, plot)], game's choice, techs, map) per city and turn."""
    rows = {}
    for f in sorted((ROOT / "data/probe").glob(f"{game}_p*.jsonl")):
        for line in open(f):
            r = json.loads(line)
            rows.setdefault(r["turn"], []).append(r)
    maps = {}
    for turn in sorted(rows):
        if turn < 3 or turn > 60:
            continue
        mt = turn
        if mt not in maps:
            maps[mt] = map_at(game, mt)
        mj = maps[mt]
        if mj is None:
            continue
        m = Map(mj)
        plots = {(p["x"], p["y"]): p for p in mj["plots"]}
        owned_any = {xy for xy, p in plots.items() if p["owner"] is not None}
        for r in rows[turn]:
            techs = {t[0] for t in r["techs"] if t[3]}
            for c in r["cities"]:
                nxt = c.get("nextPlot")
                if not nxt or nxt[0] < 0:
                    continue
                own = {(t[0], t[1]) for t in c["tiles"]}
                center = (c["x"], c["y"])
                cands = set()
                for xy in own:
                    for n in m.neighbours(*xy):
                        if n not in owned_any and m.distance(center, n) <= 5:
                            cands.add(n)
                if tuple(nxt) not in cands or len(cands) < 2:
                    continue
                yield turn, r["player"], center, [(xy, plots[xy]) for xy in sorted(cands)], tuple(nxt), techs, m


def score(rules, center, xy, p, techs, m, w):
    d = m.distance(center, xy)
    cost = w["dist"] * d
    if d > 3:
        cost += w["ring"]
    res = p["resource"] if p["resource"] and rules.resource_revealed(p["resource"], techs) else None
    if res:
        cost += w["res"]
    elif p["feature"] and rules.q("select 1 from Features where FeatureType=? and NaturalWonder=1", p["feature"]):
        cost += w["nw"]
    water = p["terrain"] in ("TERRAIN_COAST", "TERRAIN_OCEAN") or p["terrain"].endswith("MOUNTAIN")
    if water:
        cost += w["water"]
    y = tile_yields(rules, p, techs=tuple(techs))
    cost += w["yield"] * sum(y.values())
    return cost


def evaluate(all_cases, w, show=0):
    hit = n = ties = 0
    misses = []
    for game, rules, (turn, pl, center, cands, nxt, techs, m) in all_cases:
        s = {xy: score(rules, center, xy, p, techs, m, w) for xy, p in cands}
        best = min(s.values())
        top = [xy for xy, v in s.items() if v == best]
        n += 1
        if nxt in top:
            hit += 1
            ties += len(top) > 1
        else:
            misses.append((game, turn, pl, center, nxt, s[nxt], top, best))
    for mm in misses[:show]:
        print("   miss", mm)
    return hit, n, ties


def main():
    games = sys.argv[1:] or ["rome1", "rome2", "auto3", "auto4"]
    all_cases = []
    for g in games:
        rules = Rules(str(ROOT / f"data/rules/{RULES.get(g, 'gameplay_rome1')}.sqlite"))
        cs = list(cases(g))
        print(g, len(cs), "cases")
        all_cases += [(g, rules, c) for c in cs]
    gp = dict(all_cases[0][1].q("select Name, Value from GlobalParameters where Name like 'PLOT_INFLUENCE%'"))
    base = {"dist": float(gp["PLOT_INFLUENCE_DISTANCE_MULTIPLIER"]), "ring": float(gp["PLOT_INFLUENCE_RING_COST"]),
            "res": float(gp["PLOT_INFLUENCE_RESOURCE_COST"]), "nw": float(gp["PLOT_INFLUENCE_NW_COST"]),
            "water": float(gp["PLOT_INFLUENCE_WATER_COST"]), "yield": float(gp["PLOT_INFLUENCE_YIELD_POINT_COST"])}
    print("parameters", base)
    for label, w in [("as in the database", base),
                     ("distance halved (DIVISOR 2)", dict(base, dist=base["dist"] / 2)),
                     ("yield x10", dict(base, **{"yield": base["yield"] * 10}))]:
        hit, n, ties = evaluate(all_cases, w, show=5 if label == "as in the database" else 0)
        print(f"{label}: {hit}/{n} chosen plot among the best ({ties} with ties)")


if __name__ == "__main__":
    main()
