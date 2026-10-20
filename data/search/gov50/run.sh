#!/bin/bash
# Gran Colombia 10/19, T50: the best plans with the Audience Chamber (tall) and with the Ancestral Hall (wide) built by
# T50, the only requirement each (5000); score = food + production summed to T50. From the recorded play, iterated
# rollout 25 minutes, 6 threads each, on Windows (optimizer/scripts/win_run.sh; fetch the results when done).
cd "$(dirname "$0")/../../.."
C="--turn 50 --ext 0 --allow-military --team --require-weight 5000 --threads 6 --minutes 25 --window 50 --passes 20 --kick-to 45 --init record"
for v in tall:BUILDING_GOV_TALL wide:BUILDING_GOV_WIDE; do
  name=${v%%:*}; b=${v#*:}
  optimizer/scripts/win_run.sh start gov50_$name irollout data/games/gran_colombia_20261019.json $C --require $b:1 --seed 1 --out data/search/gov50/$name.json
done
