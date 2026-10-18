#!/bin/bash
R=$(cd "$(dirname "$0")/../../.." && pwd)
BIN="$R/optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt"
# France 10/14: horsemen by T43, with Castles, Mercenaries and a Great General as near-hard requirements
# (horses unlimited: given by allies). Two starts: the recorded play and the first (requirement-breaking) result.
cd "$R/optimizer"
D=$R/data/search/mil
C="--turn 43 --ext 0 --allow-military --team --require TECH_CASTLES:1,CIVIC_MERCENARIES:1,GREAT_PERSON_CLASS_GENERAL:1 --require-weight 5000 --reward UNIT_HORSEMAN:200"
one() {
  name=$1; init=$2
  nice "$BIN" rollout ../data/games/france_20261014.json $C --threads 6 --minutes 20 --quiet --init $init --out $D/${name}_ro.json > $D/${name}_ro.log 2>&1
  nice "$BIN" irollout ../data/games/france_20261014.json $C --threads 6 --minutes 40 --window 8 --init $D/${name}_ro.json --out $D/${name}_ir.json > $D/${name}_ir.log 2>&1
  echo done > $D/${name}.done
}
one rec2 $D/rec_ir.json &
one prev2 $D/prev_ir.json &
wait
