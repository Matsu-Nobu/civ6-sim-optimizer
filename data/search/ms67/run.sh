#!/bin/bash
R=$(cd "$(dirname "$0")/../../.." && pwd)
BIN="$R/optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt"
# France 10/14, own cities only (no conquest): Castles, Mercenaries, a Great General by T44; Military Science, a
# Renaissance-or-later Great General (one that leads industrial units) 2 siege towers and the Terracotta Army by T67 required (5000 each), horsemen trained by T44 +200 each; F+P to T67. Allies = Australia (player 1, from the
# record's routes): research alliance route science after Civil Service. No settling within 3 of other players' cities.
# Rollout to refit to the T67 horizon, then iterated rollout with whole-rest repair (kicks up to T55); restarted from
# the saved best if the process dies (the occasional glibc heap error).
cd "$R/optimizer"
D=$R/data/search/ms67
B="$BIN"
C="--turn 67 --ext 0 --allow-military --team --require TECH_CASTLES:1@44,CIVIC_MERCENARIES:1@44,GREAT_PERSON_CLASS_GENERAL:1@44,TECH_MILITARY_SCIENCE:1,GREAT_PERSON_CLASS_GENERAL>ERA_RENAISSANCE:1,UNIT_SIEGE_TOWER:2,BUILDING_TERRACOTTA_ARMY:1 --require-weight 5000 --reward UNIT_HORSEMAN:200@44"
one() {
  name=$1; init=$2; seed=$3
  nice $B rollout ../data/games/france_20261014.json $C --threads 8 --minutes 5 --quiet --init $init --out $D/$name.json > $D/${name}_ro.log 2>&1
  end=$(( $(date +%s) + 80*60 ))
  n=0
  while [ $(date +%s) -lt $end ]; do
    left=$(( (end - $(date +%s)) / 60 )); [ $left -lt 2 ] && break
    nice $B irollout ../data/games/france_20261014.json $C --threads 8 --minutes $left --window 80 --passes 20 --kick-to 55 --seed $((seed + n)) --init $D/$name.json --out $D/$name.json >> $D/${name}_ir.log 2>&1
    grep -q '"best"' <(tail -3 $D/${name}_ir.log) && break
    n=$((n+1)); echo "restart $n" >> $D/${name}_ir.log
  done
  echo done > $D/$name.done
}
one a $D/a0.json 11 &
one b $D/b0.json 21 &
wait
