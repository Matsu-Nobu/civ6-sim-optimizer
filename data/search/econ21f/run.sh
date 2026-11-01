#!/bin/bash
# 10/31: the option model learned again under the fixed score (harvests counted as used, a 20-turn extension), then
# the econ21d / econ21e searches again with it. Rows: civ6opt collect around the two best games (random decisions, up
# to 4 other options each, played to the end: every kind of option, harvests too, whatever the old model ranks).
# The model ranks the options the online search tries and plays the extension. Steps are skipped when done.
B=optimizer/target/release/civ6opt
G=data/games/rome_20261021.json
D=data/search/econ21f
OLD="--turn 65 --ext 20 --ext-model --fallback-model data/search/learn/aa_m3.json --team --ally-supply --sum-floor 2674,1842,10"
NEW="--turn 65 --ext 20 --ext-model --fallback-model $D/m1.json --team --ally-supply --sum-floor 2674,1842,10"
[ -f $D/collect1.tsv ] || $B collect $G $OLD --plans data/search/econ21d/online2.json,data/search/econ21e/online8.json --plays 4000 --alts 4 --threads 16 --out $D/collect1.tsv 2> $D/collect1.err
echo "collect $(date +%H:%M) $(wc -l < $D/collect1.tsv) rows" >> $D/run.log
[ -f $D/m1.json ] || .venv/bin/python tools/learn_train.py $D/collect1.tsv --out $D/m1.json --rounds 300 --threads 16 --objective rank --target delta --no-eval > $D/m1.train.log 2>&1
echo "model $(date +%H:%M)" >> $D/run.log
score() { $B run $G $NEW --genome "$1" 2>/dev/null | tail -1 | python3 -c "import json,sys; print(json.load(sys.stdin)['score'])"; }
echo "record $(score record)" >> $D/run.log
for start in d:data/search/econ21d/online2.json e:data/search/econ21e/online8.json; do
  tag=${start%%:*}; prev=${start#*:}; ps=$(score $prev)
  echo "start $tag $(date +%H:%M) $ps" >> $D/run.log
  for i in $(seq 1 12); do
    [ -f $D/${tag}_online$i.json ] || $B online $G $NEW --guide $prev --top 6 --threads 16 --out $D/${tag}_online$i.json > /dev/null 2> $D/${tag}_online$i.err
    s=$(score $D/${tag}_online$i.json); echo "$tag online$i $(date +%H:%M) $s" >> $D/run.log
    python3 -c "import sys; sys.exit(0 if float(sys.argv[2]) > float(sys.argv[1]) + 1 else 1)" $ps $s || break
    prev=$D/${tag}_online$i.json; ps=$s
  done
  echo "done $tag $(date +%H:%M) best $prev $ps" >> $D/run.log
done
