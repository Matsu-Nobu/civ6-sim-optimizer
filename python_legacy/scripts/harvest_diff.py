"""Harvest (chop / resource harvest) amounts: the extra production toward the current item on a turn a feature or
resource disappears from a city's plot, vs production.harvest_yield. Food harvests are not checked here.
Usage: python3 python_legacy/scripts/harvest_diff.py <rules> <export.jsonl> <probe.jsonl> [--offset N]"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.amenities import happiness  # noqa: E402
from civ6sim.modifiers import engine_for  # noqa: E402
from civ6sim.production import harvest_yield, item_production_percent  # noqa: E402
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.yields import city_yield_percents, percent_modifiers  # noqa: E402
from civ6sim.envs import player_of  # noqa: E402

r = Rules(sys.argv[1])
args = sys.argv[4:]
off = int(args[args.index("--offset") + 1]) if "--offset" in args else 0
rows = {d["turn"]: d for d in map(json.loads, open(sys.argv[2]))}
ok = bad = 0
for line in open(sys.argv[3]):
    pr = json.loads(line)
    d, n = rows.get(pr["turn"] + off), rows.get(pr["turn"] + off + 1)
    if not d or not n:
        continue
    eng = engine_for(r, player_of(d), d["cities"])
    for pc in pr["cities"]:
        name = pc["name"].replace("LOC_CITY_NAME_", "")
        c = next((x for x in d["cities"] if x["name"] == name), None)
        c2 = next((x for x in n["cities"] if x["name"] == name), None)
        if not c or not c2:
            continue
        item = c["production"]["current"]
        if not item or c2["production"]["current"] != item:
            continue
        before = {(q["x"], q["y"]): q for q in c["owned"]}
        gone = []
        for q in c2["owned"]:
            b = before.get((q["x"], q["y"]))
            if b and not q.get("district"):
                if b["feature"] and not q["feature"]:
                    gone.append(("feature", b["feature"]))
                if b["resource"] and not q["resource"]:
                    gone.append(("resource", b["resource"]))
        if len(gone) != 1:
            continue
        kind, what = gone[0]
        g = percent_modifiers(happiness(r, pc["amenities"] - pc["amenitiesNeeded"])[0], False,
                              city_yield_percents(r, eng, c))["YIELD_PRODUCTION"]
        i = item_production_percent(r, eng, c, item)
        normal = pc["yields"]["YIELD_PRODUCTION"] * (1 + (g + i) / 100) / (1 + g / 100)
        extra = c2["production"]["progress"].get(item, 0) - c["production"]["progress"].get(item, 0) - normal
        pred = harvest_yield(r, eng, kind, what, d["player"]["techs"], d["player"]["civics"], c, "GAMESPEED_ONLINE").get("YIELD_PRODUCTION", 0)
        if pred == 0 and abs(extra) < 1:
            continue  # a food harvest or no production part
        good = abs(extra - pred) < 1.5
        ok += good
        bad += not good
        print("ok " if good else "BAD", pr["turn"], name, item, what, "extra %.2f" % extra, "pred %.2f" % pred,
              "ratio %.3f item%% %g gen%% %g" % (extra / pred if pred else 0, i, g))
print(f"match {ok} / {ok + bad}")
