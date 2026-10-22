#!/bin/bash
# 10/23 night: perturbations of the best game (online --kick): a random decision is changed to one of the model's
# next options, every decision after it is tried as always, and the game replaces the best if it ends better.
# Run from the repository root: data/search/learn/kicks.sh <best game plan> <model> <kicks> <decisions>
B=optimizer/target/release/civ6opt
G=data/games/rome_20261021.json
C="--turn 65 --ext 0 --allow-military --team --require TECH_BALLISTICS:1@65 --require-weight 20000 --reward UNIT_KNIGHT:100@65"
D=data/search/learn
best=$1
score() { python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['best'])" "$1"; }
for k in $(seq 1 $3); do
  n=$((RANDOM % $4 + 1)); r=$((RANDOM % 3))
  $B online $G $C --fallback-model $2 --guide $best --kick $n:$r --top 6 --threads 16 --out $D/kick$k.json --rows $D/rome21_gkick$k.tsv > /dev/null 2> $D/kick$k.err
  old=$(score $best); new=$(score $D/kick$k.json)
  echo "kick $k $(date +%H:%M) decision $n rank $r $(grep '^kick' $D/kick$k.err) $old -> $new" >> $D/kicks.log
  if python3 -c "import sys; sys.exit(0 if float(sys.argv[2]) > float(sys.argv[1]) + 1 else 1)" $old $new; then best=$D/kick$k.json; echo "  new best $best" >> $D/kicks.log; fi
done
echo "best $best $(score $best)" >> $D/kicks.log
