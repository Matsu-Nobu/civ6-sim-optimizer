#!/bin/bash
# Improves the value net with its own games, on the 10/21 Rome game (run from the repository root):
#   iterate.sh <first iteration> <last iteration>
# Iteration n plays a game without a plan with the lookahead of net_it<n-1>.json (net_it0 = the value6 net) for 1,
# 3 and 5 turns, saves each as a full plan (look<k>_it<n>.json), collects 1000 plays around each with valuerows
# (Windows jobs value6_<next numbers>), trains a net on all value6 jobs (net_it<n>.json, on the Windows GPU) and
# writes the scores of the three games to scores.tsv.
set -eu
D=data/search/netlook
NN=/mnt/c/Users/${WINUSER:-nobus}/civ6opt/nn
JOBS=/mnt/c/Users/${WINUSER:-nobus}/civ6opt/jobs
[ -f $D/net_it0.json ] || cp data/search/value6/net.json $D/net_it0.json
wait_job() { until grep -q done $JOBS/$1/status.txt 2>/dev/null; do sleep 5; done; }
for n in $(seq $1 $2); do
  prev=$D/net_it$((n - 1)).json
  for k in 1 3 5; do
    j=it${n}_look$k
    optimizer/scripts/win_run.sh start $j complete data/games/rome_20261021.json --turn 65 --init base --ally-supply \
      --value-net $prev --look $k --out $D/look${k}_it$n.json > /dev/null
    wait_job $j
    optimizer/scripts/win_run.sh fetch $j > /dev/null
    score=$(tr -d '\r' < $JOBS/$j/stderr.txt | grep -o '^[0-9.]*, [0-9]* decisions' | cut -d, -f1)
    printf '%s\t%s\t%s\n' $((n - 1)) $k $score >> $D/scores.tsv
  done
  for k in 1 3 5; do
    i=$(( $(ls -d $JOBS/value6_* | sed 's/.*_//' | sort -n | tail -1) + 1 ))
    optimizer/scripts/win_run.sh start value6_$i valuerows data/games/rome_20261021.json --turn 65 --plays 1000 --threads 8 \
      --ally-supply --base-after --init $D/look${k}_it$n.json --out data/search/value6/rows_$i.tsv --city-out cities.tsv > /dev/null
    wait_job value6_$i
    optimizer/scripts/win_run.sh fetch value6_$i > /dev/null
  done
  cp tools/value_net.py tools/value_compare.py $NN/
  python3 - "$n" <<'EOF'
import os, sys
n = sys.argv[1]
jobs = sorted(int(d.split("_")[1]) for d in os.listdir(f"/mnt/c/Users/{os.environ.get('WINUSER', 'nobus')}/civ6opt/jobs") if d.startswith("value6_"))
J = r"C:\Users\nobus\civ6opt\jobs\value6_"
tr = " ".join(f"'{J}{k}'" for k in jobs)
te = " ".join(f"'{J}{k}'" for k in (1, 6, 12))
lines = ["Set-Location $PSScriptRoot", f"Remove-Item -ErrorAction SilentlyContinue it{n}.done",
         rf"& ..\venv\Scripts\python.exe value_net.py --train {tr} --test {te} --epochs 40 --save net_it{n}.json 2>&1 | Out-File -Encoding utf8 it{n}.txt",
         f'"done" | Out-File it{n}.done']
open(f"/mnt/c/Users/{os.environ.get('WINUSER', 'nobus')}/civ6opt/nn/it{n}.ps1", "w").write("\r\n".join(lines) + "\r\n")
EOF
  powershell.exe -NoProfile -Command "Start-Process powershell.exe -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File','C:\Users\nobus\civ6opt\nn\it$n.ps1' -WindowStyle Hidden"
  until [ -f $NN/it$n.done ]; do sleep 10; done
  cp $NN/net_it$n.json $D/net_it$n.json
  echo "iteration $n done $(date +%T)"
done
