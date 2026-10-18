#!/bin/bash
# Prints, for the recorded play and each search result: food + production to T35, the military strength owned at T35,
# the units, and the research, civic and production orders.
R=$(cd "$(dirname "$0")/../../.." && pwd)
# the requirements of run3.sh, reported per plan
REQ="--require DISTRICT_GOVERNMENT:1,DISTRICT_ENCAMPMENT:1,BUILDING_MONUMENT:1,UNIT_SLINGER:2@25,cities:2@12,cities:3@20 --require-weight 5000"
BIN="$R/optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt"
D=$R/data/search/france18
cd "$D"
for v in ${@:-record fixed_rec fixed_a free_rec free_a}; do
  g=$v.json; [ $v = record ] && g=record
  "$BIN" run "$R/data/games/france_20261018.json" --turn 35 --ext 0 --team --allow-military $REQ --genome "$g" 2> "sum_$v.err" > "sum_$v.out"
  "$BIN" decisions "$R/data/games/france_20261018.json" --turn 35 --ext 0 --team --allow-military --genome "$g" 2>/dev/null > "sum_$v.dec"
  python3 - "$v" "$R/data/rules/names_ja.tsv" <<'PY'
import json, re, sys
v = sys.argv[1]
JA = dict(l.rstrip("\n").split("\t") for l in open(sys.argv[2]))
ja = lambda t: JA.get(t, t)
S = {"UNIT_HORSEMAN": 36, "UNIT_HEAVY_CHARIOT": 36, "UNIT_SPEARMAN": 29, "UNIT_ARCHER": 25, "UNIT_WARRIOR": 24, "UNIT_SLINGER": 15}
T = [j for j in (json.loads(l) for l in open(f"sum_{v}.out")) if "turn" in j]
fp = sum(j["output"][0] + j["output"][1] for j in T)
u = {x.split(":")[0]: int(x.split(":")[1]) for x in T[-1]["units_trained"]}
u["UNIT_WARRIOR"] = u.get("UNIT_WARRIOR", 0) + 1  # the starting warrior
st = sum(S[k] * n for k, n in u.items() if k in S)
city = lambda n: next((j["turn"] for j in T if len(j["cities"]) >= n), None)
print(f"== {v}: F+P {fp:.0f}, archers {u.get('UNIT_ARCHER',0)} horsemen {u.get('UNIT_HORSEMAN',0)}, cities T{city(2)} T{city(3)}, gold T35 {T[-1]['gold']:.0f}")
print("  ", " ".join(l.strip() for l in open(f"sum_{v}.err") if l.startswith("require")))
print("   units", {ja(k): n for k, n in u.items()})
dec = [json.loads(l) for l in open(f"sum_{v}.dec")]
print("   research", " ".join(f"T{d['turn']}:{ja(d['choice'])}" for d in dec if d["key"][0] == "research"))
print("   civic", " ".join(f"T{d['turn']}:{ja(d['choice'])}" for d in dec if d["key"][0] == "civic"))
print("   policy", " ".join(f"T{d['turn']}:{ja(d['choice'])}" for d in dec if d["key"][0] == "policy" and d["choice"] != "NONE"))
for c in dict.fromkeys(d["key"][1] for d in dec if d["key"][0] == "prod"):
    print("  ", c, " ".join(f"T{d['turn']}:{ja(d['choice'])}" for d in dec if d["key"][0] == "prod" and d["key"][1] == c))
print("   buys", [(d["turn"], d["choice"]) for d in dec if d["key"][0] == "buy" and d["choice"] != ["none"]])
PY
done
