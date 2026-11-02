"""Gaps between the simulator's city yields and the game's in recorded games, with the conditions of each city and
turn, to find the effects the simulator lacks: for each condition (a policy, the government, a governor in the city, a
building, a district, a city-state the player is suzerain of, a trade route), the mean gap of the city-turns that
have it against those that do not.

  yield_gaps.py <game data name>... [--from 20] [--yield food|production|gold|science|culture]

The simulator's yields are those of `civ6opt validate --team --ally-supply` (the options the searches use) on each
recorded turn (CIV6OPT_TRACE=cityyields); the game's are the save's city yields of the same turn.
"""
import argparse
import collections
import json
import os
import re
import sqlite3

import civ6opt

YI = {"food": 0, "production": 1, "gold": 2, "science": 3, "culture": 4}
YN = {"food": "YIELD_FOOD", "production": "YIELD_PRODUCTION", "gold": "YIELD_GOLD", "science": "YIELD_SCIENCE",
      "culture": "YIELD_CULTURE"}


def conditions(r, i, c, cs_kind):
    p = r["player"]
    out = {f"policy {x}" for x in p.get("policies") or []}
    out.add(f"government {p.get('government')}")
    out |= {f"building {b}" for b in c.get("buildings") or []}
    ds = c.get("districts") or []
    out |= {f"district {d['type'] if isinstance(d, dict) else d}" for d in ds}
    for g in p.get("governors") or []:
        if isinstance(g, dict) and g.get("cityId") == c.get("id"):
            out.add(f"governor {g['type']}")
            out |= {f"promotion {x}" for x in g.get("promotions") or []}
    kinds = collections.Counter()
    for e in p.get("envoys") or []:
        if isinstance(e, dict) and e.get("suzerain"):
            out.add(f"suzerain {e.get('civilization')}")
        if isinstance(e, dict) and e.get("count", 0) >= 1 and e.get("civilization") in cs_kind:
            kinds[cs_kind[e["civilization"]]] += 1
    # the city-states of each kind with an envoy of the player, with the building their bonus goes to
    for k, n in kinds.items():
        out.add(f"city-states {k} {n}")
        for b in c.get("buildings") or []:
            out.add(f"city-states {k} {n} with {b}")
    if c.get("capital"):
        out.add("capital")
    me = p.get("playerId")
    for t in r.get("tradeRoutes") or []:
        if t["origin"].get("playerId") == me and t["origin"]["name"] == c["name"]:
            out.add("route out " + ("domestic" if t["destination"].get("playerId") == me else "international"))
        if t["destination"].get("playerId") == me and t["destination"]["name"] == c["name"]:
            out.add("route in")
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("games", nargs="+")
    ap.add_argument("--from", dest="start", type=int, default=20)
    ap.add_argument("--yield", dest="y", default="food")
    # --own: only the cities the player founded (named from its civilization's list), not those it took
    ap.add_argument("--own", action="store_true")
    a = ap.parse_args()
    data = []
    for g in a.games:
        path = civ6opt.ROOT / "data" / "games" / f"{g}.json"
        G = json.load(open(path))
        rows = {r["turn"]: r for r in G["rows"]}
        civ = G["rows"][0]["player"]["civilization"]
        db = sqlite3.connect(civ6opt.ROOT / "data" / "rules" / f"{g}.sqlite")
        cs_kind = {civ: kind.replace("LEADER_MINOR_CIV_", "") for civ, kind in db.execute(
            "select cl.CivilizationType, l.InheritFrom from CivilizationLeaders cl join Leaders l on l.LeaderType = cl.LeaderType "
            "where l.InheritFrom like 'LEADER_MINOR_CIV_%'")}
        own_names = {n.replace("LOC_CITY_NAME_", "") for (n,) in db.execute("select CityName from CityNames where CivilizationType=?", (civ,))}
        out = civ6opt.run(["validate", path, "--turn", "66", "--from", a.start, "--team", "--ally-supply"], capture_output=True, text=True,
                          env={**os.environ, "CIV6OPT_TRACE": "cityyields"}).stderr
        for m in re.finditer(r"CY T(\d+) (\S+) \[([^\]]*)\]", out):
            t, name, ys = int(m.group(1)), m.group(2), [float(x) for x in m.group(3).split(",")]
            r = rows.get(t)
            if not r:
                continue
            if a.own and name not in own_names:
                continue
            i = next((k for k, c in enumerate(r["cities"]) if c["name"] == name), None)
            if i is None:
                continue
            if not r.get("cityYields") or i >= len(r["cityYields"]) or not r["cityYields"][i]:
                continue
            gy = r["cityYields"][i]["modified"]
            data.append((g, t, name, ys[YI[a.y]] - gy[YN[a.y]], conditions(r, i, r["cities"][i], cs_kind)))
    gaps = [d[3] for d in data]
    print(f"{len(data)} city-turns; mean gap (simulator - game) {sum(gaps) / len(gaps):.2f}; "
          f"mean absolute gap {sum(abs(x) for x in gaps) / len(gaps):.2f}; off by more than 0.5: {sum(abs(x) > 0.5 for x in gaps)}")
    stat = collections.defaultdict(list)
    for d in data:
        for c in d[4]:
            stat[c].append(d[3])
    rows = []
    for c, v in stat.items():
        rest = [d[3] for d in data if c not in d[4]]
        if len(v) < 8 or not rest:
            continue
        rows.append((sum(v) / len(v) - sum(rest) / len(rest), len(v), sum(v) / len(v), c))
    rows.sort()
    print("difference of mean gap (with - without), city-turns with it, mean gap with it, condition")
    for x in rows[:15] + [None] + rows[-8:]:
        print("   ..." if x is None else f"{x[0]:7.2f} {x[1]:5d} {x[2]:7.2f}  {x[3]}")


if __name__ == "__main__":
    main()
