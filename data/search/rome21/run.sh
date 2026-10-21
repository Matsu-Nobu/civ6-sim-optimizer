#!/bin/bash
# Rome 10/21, T65: score = food + production summed to T65 + 100 per knight by T65; Ballistics done by the end of T65
# (cuirassiers from T66), 20000 if not. From the recorded play (record_search: all its choices, under the model's rules), iterated rollout on Windows
# (optimizer/scripts/win_run.sh; fetch the results when done). $1: minutes (default 40), $2: job name suffix.
cd "$(dirname "$0")/../../.."
M=${1:-40}
C="--turn 65 --ext 0 --allow-military --team --require TECH_BALLISTICS:1@65 --require-weight 20000 --reward UNIT_KNIGHT:100@65 --threads 12 --minutes $M --window 65 --passes 20 --kick-to 60 --init record_search"
optimizer/scripts/win_run.sh start rome21_${2:-main} irollout data/games/rome_20261021.json $C --seed 1 --out data/search/rome21/${2:-main}.json
