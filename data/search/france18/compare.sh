#!/bin/bash
# Runs each one-change variant of the recorded play to T35 and prints F+P, military units and techs.
# The plan files are named record_*.json and given by a path that starts with "record", so that civ6opt also keeps the recorded citizens' tiles, as --genome record does.
R=$(cd "$(dirname "$0")/../../.." && pwd)
BIN="$R/optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt"
D=$R/data/search/france18
cd "$D"
for v in base hbr_first hbr_early mt_first builder; do
  "$BIN" run "$R/data/games/france_20261018.json" --turn 35 --ext 0 --team --allow-military --genome "record_$v.json" 2>/dev/null > "record_$v.out"
  python3 - "record_$v.out" "$v" <<'PY'
import json, sys
L = [json.loads(l) for l in open(sys.argv[1])]
T = [j for j in L if "turn" in j]
fp = sum(j["output"][0] + j["output"][1] for j in T)
def units(j, kind): return sum(int(u.split(":")[1]) for u in j["units_trained"] if u.startswith(kind + ":"))
first = next((j["turn"] for j in T if units(j, "UNIT_HORSEMAN") > 0), None)
e = T[-1]
mil = {k: units(e, k) for k in ("UNIT_HORSEMAN", "UNIT_ARCHER", "UNIT_SLINGER", "UNIT_SETTLER", "UNIT_BUILDER")}
print(f"{sys.argv[2]:22s} F+P {fp:7.1f} first horseman T{first} cities {len(e['cities'])} {mil} research {e['research']} civic {e['civic']}")
PY
done
