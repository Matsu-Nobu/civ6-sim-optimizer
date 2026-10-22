#!/bin/bash
# 10/23 night: guided rollout in turns. Iteration i trains on every row so far, then plays an online game guided by
# the best game so far (online --guide: it scores at least as the guide). Stops after <last>, or after 3 iterations
# in a row that do not improve the best by more than 1 point.
# Run from the repository root: data/search/learn/iterate_guided.sh <first> <last> <best game plan>
set -e
B=optimizer/target/release/civ6opt
G=data/games/rome_20261021.json
C="--turn 65 --ext 0 --allow-military --team --require TECH_BALLISTICS:1@65 --require-weight 20000 --reward UNIT_KNIGHT:100@65"
D=data/search/learn
best=$3
score() { python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['best'])" "$1"; }
stale=0
for i in $(seq $1 $2); do
  rows="$D/rome21_collect3.tsv $(ls $D/rome21_online*.tsv $D/rome21_guide*.tsv | tr '\n' ' ')"
  .venv/bin/python tools/learn_train.py $rows --out $D/g$i.json --rounds 300 --threads 16 --objective rank --target delta --no-eval > $D/g$i.train.log 2>&1
  $B online $G $C --fallback-model $D/g$i.json --guide $best --top 6 --threads 16 --out $D/guide$i.json --rows $D/rome21_guide$i.tsv > $D/guide$i.out 2> $D/guide$i.err
  old=$(score $best); new=$(score $D/guide$i.json)
  echo "iteration $i $(date +%H:%M) guide $old -> $new $(grep check $D/guide$i.err)" >> $D/iterate_guided.log
  if python3 -c "import sys; sys.exit(0 if float(sys.argv[2]) > float(sys.argv[1]) + 1 else 1)" $old $new; then best=$D/guide$i.json; stale=0; else stale=$((stale+1)); fi
  [ $stale -ge 3 ] && break
done
echo "best $best $(score $best)" >> $D/iterate_guided.log
