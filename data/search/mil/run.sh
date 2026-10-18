#!/bin/bash
R=$(cd "$(dirname "$0")/../../.." && pwd)
BIN="$R/optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt"
# France 10/14: horsemen by T43 with Castles and Mercenaries done by T43 (rollout, then iterated rollout)
cd "$R/optimizer"
D=$R/data/search/mil
C="--turn 43 --ext 0 --allow-military --team --require TECH_CASTLES:1,CIVIC_MERCENARIES:1 --reward UNIT_HORSEMAN:200"
nice "$BIN" rollout ../data/games/france_20261014.json $C --threads 12 --minutes 25 --quiet --init record_wide --out $D/ro.json > $D/ro.log 2>&1
nice "$BIN" irollout ../data/games/france_20261014.json $C --threads 12 --minutes 35 --window 8 --init $D/ro.json --out $D/ir.json > $D/ir.log 2>&1
echo done >> $D/ro.log
