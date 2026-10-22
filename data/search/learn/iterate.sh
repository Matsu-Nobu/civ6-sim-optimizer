#!/bin/bash
# 10/23 night: rollout and retraining in turns (docs/design/learning.md). Iteration i trains on the collected rows
# and every online game's rows so far, then plays an online game with that model (top 6 options per decision).
# Run from the repository root: data/search/learn/iterate.sh <first iteration> <last iteration>
set -e
B=optimizer/target/release/civ6opt
G=data/games/rome_20261021.json
C="--turn 65 --ext 0 --allow-military --team --require TECH_BALLISTICS:1@65 --require-weight 20000 --reward UNIT_KNIGHT:100@65"
D=data/search/learn
for i in $(seq $1 $2); do
  rows="$D/rome21_collect3.tsv $(ls $D/rome21_online*.tsv | tr '\n' ' ')"
  .venv/bin/python tools/learn_train.py $rows --out $D/it$i.json --rounds 300 --threads 16 --objective rank --target delta > $D/it$i.train.log 2>&1
  $B online $G $C --fallback-model $D/it$i.json --top 6 --threads 16 --out $D/online_it$i.json --rows $D/rome21_online_it$i.tsv > $D/online_it$i.out 2> $D/online_it$i.err
  echo "iteration $i $(date +%H:%M) $(grep check $D/online_it$i.err) $(grep -E 'require|reward' $D/online_it$i.err | tr '\n' ' ')" >> $D/iterate.log
done
