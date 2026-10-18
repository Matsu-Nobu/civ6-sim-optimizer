#!/bin/bash
R=$(cd "$(dirname "$0")/../../.." && pwd)
BIN="$R/optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt"
# France 10/14, deadline T43: Castles, Mercenaries and a Great General required (5000 each); horsemen +200 each
# (horses unlimited: allies give them). Start: the plan that meets the requirements (feasb43).
cd "$R/optimizer"
D=$R/data/search/mil
C="--turn 43 --ext 0 --allow-military --team --require TECH_CASTLES:1,CIVIC_MERCENARIES:1,GREAT_PERSON_CLASS_GENERAL:1 --require-weight 5000 --reward UNIT_HORSEMAN:200"
nice "$BIN" rollout ../data/games/france_20261014.json $C --threads 12 --minutes 15 --quiet --init $D/feasb43.json --out $D/max.json > $D/max_ro.log 2>&1
nice "$BIN" irollout ../data/games/france_20261014.json $C --threads 12 --minutes 35 --window 10 --init $D/max.json --out $D/max.json > $D/max_ir.log 2>&1
echo done > $D/max.done
