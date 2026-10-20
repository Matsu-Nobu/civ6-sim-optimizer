"""Compares a full replay of the recorded play (civ6opt run --genome record) with the record, turn by turn.

  replay_check.py <game data .json> <replay output> [--cities]

The replay output is civ6opt run's stdout (one state per turn, labelled with the turn just played). The record's
state after turn t is the game data's row of turn t + 1. Prints, for every turn, the player's food, production,
science and culture (replay / record), cities, population, techs, civics and government where they differ; then, for
each city, the first turn its population, production item or worked tiles differ (--cities: every city every turn).
"""
import json
import sys

game, out = sys.argv[1], sys.argv[2]
rows = {r["turn"]: r for r in json.load(open(game))["rows"]}
sim = {}
for line in open(out):
    try:
        d = json.loads(line)
    except json.JSONDecodeError:
        continue
    if "turn" in d:
        sim[d["turn"]] = d


def rec_city(c):
    return {"pop": c["population"], "item": c["production"].get("current"),
            "worked": sorted((w["x"], w["y"]) for w in c["worked"])}


first = {}
print("turn  food  production  science  culture  | differences")
for t in sorted(sim):
    if t + 1 not in rows:
        break
    s, r = sim[t], rows[t + 1]
    o, y = s["output"], r["player"]["yields"]
    notes = []
    if len(s["cities"]) != len(r["cities"]):
        notes.append(f"cities {len(s['cities'])}/{len(r['cities'])}")
    sp, rp = sum(c["pop"] for c in s["cities"]), sum(c["population"] for c in r["cities"])
    if sp != rp:
        notes.append(f"population {sp}/{rp}")
    if s["techs"] != len(r["player"]["techs"]):
        notes.append(f"techs {s['techs']}/{len(r['player']['techs'])}")
    if s["civics"] != len(r["player"]["civics"]):
        notes.append(f"civics {s['civics']}/{len(r['player']['civics'])}")
    if s["government"] != r["player"]["government"]:
        notes.append(f"government {s['government']}/{r['player']['government']}")
    print(f"T{t:<3} {o[0]:4.0f}/{y.get('YIELD_FOOD', 0):<4.0f} {o[1]:4.0f}/{y.get('YIELD_PRODUCTION', 0):<4.0f}"
          f" {o[3]:4.0f}/{y.get('YIELD_SCIENCE', 0):<4.0f} {o[4]:4.0f}/{y.get('YIELD_CULTURE', 0):<4.0f} | {', '.join(notes)}")
    rc = {c["name"]: rec_city(c) for c in r["cities"]}
    # the worked tiles and the item of turn t + 1 (the replay's next state; the record's row t + 1 shows them at the
    # start of that turn); a replay item of None (one finished, the next not yet chosen) is not compared
    nxt = {c["name"]: c for c in sim.get(t + 1, {"cities": []})["cities"]}
    for c in s["cities"]:
        name = c["name"]
        if name not in rc:
            first.setdefault((name, "exists"), (t, "only in the replay"))
            continue
        rcc = rc[name]
        n = nxt.get(name)
        checks = [("pop", c["pop"], rcc["pop"])]
        if n is not None:
            checks.append(("worked", sorted(tuple(xy) for xy in n["worked"]), rcc["worked"]))
            if n["item"] is not None:
                checks.append(("item", n["item"], rcc["item"]))
        for k, a, b in checks:
            if a != b:
                first.setdefault((name, k), (t, f"{a} / {b}"))
                if "--cities" in sys.argv:
                    print(f"      {name} {k}: {a} / {b}")
    for name in rc:
        # a city founded during turn t is in the record's row t + 1 and in the replay's state after turn t + 1 (both
        # produce from turn t + 1 on)
        if name not in [c["name"] for c in s["cities"]] and name not in nxt:
            first.setdefault((name, "exists"), (t, "only in the record"))
print("\nfirst differences by city (replay / record):")
for (name, k), (t, v) in sorted(first.items(), key=lambda x: x[1][0]):
    print(f"  T{t:<3} {name} {k}: {v if k != 'worked' else ''}")
