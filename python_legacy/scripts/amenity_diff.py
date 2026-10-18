"""Amenity supply: game vs ours per city, most common differences. Usage: amenity_diff.py <rules> <export> <probe> [--offset N]"""
import json
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.amenities import amenity_supply  # noqa: E402
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.envs import player_of  # noqa: E402


def main():
    r = Rules(sys.argv[1])
    args = sys.argv[4:]
    off = int(args[args.index("--offset") + 1]) if "--offset" in args else 0
    rows = {d["turn"]: d for d in map(json.loads, open(sys.argv[2]))}
    agg = Counter()
    first = {}
    for line in open(sys.argv[3]):
        pr = json.loads(line)
        d = rows.get(pr["turn"] + off)
        if not d:
            continue
        player = player_of(d)
        sup = amenity_supply(r, d["cities"], player)
        for pc in pr["cities"]:
            c = next((x for x in d["cities"] if (x["center"]["x"], x["center"]["y"]) == (pc["x"], pc["y"])), None)
            if not c:
                continue
            diff = round(pc["amenities"] - sup[c["name"]], 2)
            agg[(c["name"], diff)] += 1
            first.setdefault((c["name"], diff), pr["turn"])
    for (n, dv), k in sorted(agg.items(), key=lambda x: -x[1])[:30]:
        print(f"{k:4d} {n:22s} game-ours {dv:+g}  from T{first[(n, dv)]}")


if __name__ == "__main__":
    main()
