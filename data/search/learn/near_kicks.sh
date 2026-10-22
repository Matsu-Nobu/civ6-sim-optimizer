#!/bin/bash
# 10/23 night: perturbations from near misses. A guided game on the best game lists the options that came within
# 150 points of each decision's best try (online --near); they are kicked in turn, the closest first, each game
# trying every decision after the kick as always. A game that ends better becomes the best, and the list is made
# again from it. Run from the repository root: data/search/learn/near_kicks.sh <best game plan> <model> <rounds>
B=optimizer/target/release/civ6opt
G=data/games/rome_20261021.json
# EXTRA: more options for every game (e.g. --ally-supply)
C="--turn 65 --ext 0 --allow-military --team --require TECH_BALLISTICS:1@65 --require-weight 20000 --reward UNIT_KNIGHT:100@65 ${EXTRA:-}"
D=data/search/learn
best=$1; model=$2
# a game's score under the current rules and options (a plan file's own "best" may come from older rules)
score() { $B run $G $C --genome "$1" 2>/dev/null | tail -1 | python3 -c "import json,sys; print(json.load(sys.stdin)['score'])"; }
k=0
for round in $(seq 1 $3); do
  $B online $G $C --fallback-model $model --guide $best --top 6 --threads 16 --near 150 --out $D/near_base$round.json > /dev/null 2> $D/near_base$round.err
  # the listed decisions are numbered in this game, which scores at least as the guide (ties may take another
  # option of the same score): it becomes the best the kicks follow
  best=$D/near_base$round.json
  grep '^near ' $D/near_base$round.err | sort -k3 -g | head -40 > $D/near_list$round.txt
  echo "round $round $(date +%H:%M) best $(score $best), $(wc -l < $D/near_list$round.txt) near misses" >> $D/near_kicks.log
  improved=0
  while read -r _ n gap choice; do
    k=$((k+1))
    $B online $G $C --fallback-model $model --guide $best --kick "$n:$choice" --top 6 --threads 16 --out $D/nk$k.json --rows $D/rome21_gnk$k.tsv > /dev/null 2> $D/nk$k.err
    old=$(score $best); new=$(score $D/nk$k.json)
    echo "  kick $k decision $n gap $gap $(grep '^kick' $D/nk$k.err | cut -c1-80) $old -> $new" >> $D/near_kicks.log
    if python3 -c "import sys; sys.exit(0 if float(sys.argv[2]) > float(sys.argv[1]) + 1 else 1)" $old $new; then best=$D/nk$k.json; improved=1; echo "  new best $best" >> $D/near_kicks.log; break; fi
  done < $D/near_list$round.txt
  [ $improved = 0 ] && break
done
echo "best $best $(score $best)" >> $D/near_kicks.log
