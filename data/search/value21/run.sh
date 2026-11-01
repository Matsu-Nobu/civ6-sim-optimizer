#!/bin/bash
# 11/01: rows for comparing models of a state's value (linear per turn vs LightGBM), around the best econ21f game:
# each play changes 1 to 5 of its decisions at random and plays to the end under the econ21f score. Then the
# comparison (tools/value_compare.py). Run from the repository root.
B=optimizer/target/release/civ6opt
C="--turn 65 --ext 20 --ext-model --fallback-model data/search/econ21f/m1.json --team --ally-supply --sum-floor 2674,1842,10"
D=data/search/value21
[ -s $D/rows1.tsv ] || $B valuerows data/games/rome_20261021.json $C --init data/search/econ21f/d_online4.json --plays 3000 --changes 5 --threads 16 --out $D/rows1.tsv 2> $D/rows1.err
echo "rows $(date +%H:%M) $(wc -l < $D/rows1.tsv)" >> $D/run.log
.venv/bin/python tools/value_compare.py $D/rows1.tsv > $D/compare1.txt 2>&1
echo "compared $(date +%H:%M)" >> $D/run.log
