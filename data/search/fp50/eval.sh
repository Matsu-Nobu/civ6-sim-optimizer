#!/bin/bash
# eval.sh <plan name> <game>: T30 food and production of a plan saved in this directory
cd "$(dirname "$0")/../../../optimizer"
O=../data/search/fp50
target/x86_64-unknown-linux-gnu/release/civ6opt run ../data/games/$2.json --turn 30 --ext 0 --allow-military --team --reward fp_min:60,food:5,production:5 --genome $O/$1.json > $O/$1.eval 2>&1
python3 - $O/$1.eval $1 <<'PY'
import json,sys
last=None
for l in open(sys.argv[1]):
    try: d=json.loads(l)
    except Exception: continue
    if 'turn' in d: last=d
    elif 'total' in d: tot=d['total']
print(sys.argv[2],'T',last['turn'],'F %.0f P %.0f'%tuple(last['output'][:2]),'cities',len(last['cities']),'pop',sum(c['pop'] for c in last['cities']),'sum F+P %.0f'%tot)
PY
