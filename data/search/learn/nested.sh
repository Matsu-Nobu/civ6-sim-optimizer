#!/bin/bash
# 10/23: nested rollout at production decisions. A guided online game on the best game lists every option tried at
# every decision with its loss against the decision's best try (online --near). At each production decision in
# turn, the three options with the smallest loss are each kicked (online --kick: taken there, every decision after
# it tried again), three games side by side. A game that ends better becomes the best; the list is then made again
# from it. Progress is saved after every decision to nested_state.txt (the best game and the next decision), so the
# script can be stopped at any time and started again: it goes on from there. A pass goes through the production
# decisions once; a pass that found a better game is followed by another from the first decision, and the script stops
# after a pass that found none (no single production change, with every decision after it tried again, improves the
# game).
# Run from the repository root: data/search/learn/nested.sh <model> [first best game, when there is no state yet]
# The game, the conditions and the folder can be given as G, C and D in the environment (default: the 10/21 Rome
# game with the knights-and-Ballistics conditions, in data/search/learn).
B=optimizer/target/release/civ6opt
G=${G:-data/games/rome_20261021.json}
C=${C:-"--turn 65 --ext 0 --allow-military --team --require TECH_BALLISTICS:1@65 --require-weight 20000 --reward UNIT_KNIGHT:100@65 --ally-supply"}
D=${D:-data/search/learn}
S=$D/nested_state.txt
model=$1
score() { $B run $G $C --genome "$1" 2>/dev/null | tail -1 | python3 -c "import json,sys; print(json.load(sys.stdin)['score'])"; }
better() { python3 -c "import sys; sys.exit(0 if float(sys.argv[2]) > float(sys.argv[1]) + 1 else 1)" "$1" "$2"; }
pass=1; improved=0
if [ -f $S ]; then source $S; else best=$2; next=1; round=0; fi
save() { printf 'best=%s\nnext=%s\nround=%s\npass=%s\nimproved=%s\n' "$best" "$next" "$round" "$pass" "$improved" > $S; }
list() {
  round=$((round+1))
  $B online $G $C --fallback-model $model --guide $best --top 6 --threads 16 --near 1e9 --out $D/nested_base$round.json > /dev/null 2> $D/nested_base$round.err
  # the listed decisions are numbered in this game, which scores at least as the guide
  best=$D/nested_base$round.json
  # production choices: a quoted item name; per decision the three smallest losses (a loss of about 0 changes nothing)
  grep '^near ' $D/nested_base$round.err | awk '$3 > 0.5 && $4 ~ /^"(UNIT|BUILDING|DISTRICT|PROJECT)_/' | sort -k2,2n -k3,3g \
    | awk '{ if (c[$2]++ < 3) print }' > $D/nested_list$round.txt
  save
  echo "list $round $(date +%H:%M) best $(score $best), $(cut -d' ' -f2 $D/nested_list$round.txt | sort -u | wc -l) production decisions from $next" >> $D/nested.log
}
[ -f $D/nested_list$round.txt ] && [ $round -gt 0 ] || list
while true; do
  n=$(awk -v n=$next '$2 >= n {print $2; exit}' $D/nested_list$round.txt)
  if [ -z "$n" ]; then
    echo "pass $pass end $(date +%H:%M): $([ $improved = 1 ] && echo improved || echo no improvement), best $(score $best)" >> $D/nested.log
    [ $improved = 0 ] && break
    pass=$((pass+1)); improved=0; next=1; save; continue
  fi
  old=$(score $best); k=0
  while read -r _ d gap choice; do
    k=$((k+1))
    $B online $G $C --fallback-model $model --guide $best --kick "$d:$choice" --top 6 --threads 5 --out $D/nested_k$k.json > /dev/null 2> $D/nested_k$k.err &
  done < <(awk -v n=$n '$2 == n' $D/nested_list$round.txt)
  wait
  top=$best; topscore=$old
  for j in $(seq 1 $k); do
    s=$(score $D/nested_k$j.json)
    echo "  decision $n $(grep '^kick' $D/nested_k$j.err | cut -c1-70) $old -> $s" >> $D/nested.log
    if better $topscore $s; then top=$D/nested_k$j.json; topscore=$s; fi
  done
  next=$((n+1))
  if [ "$top" != "$best" ]; then
    cp $top $D/nested_best$round.json; best=$D/nested_best$round.json
    echo "  new best $topscore at decision $n" >> $D/nested.log
    improved=1
    save; list
  else
    save
  fi
done
echo "done $(date +%H:%M) best $best $(score $best)" >> $D/nested.log
