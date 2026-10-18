#!/bin/bash
# Usage: save_export/export_game.sh <saveDir> <playerId> <rules.sqlite> > out.jsonl
# Passes names from the ruleset to the exporter: every type (so the parser knows mod additions), resources,
# religions, beliefs and pantheons.
set -e
q() { python3 -c "import sqlite3,sys; print(','.join(r[0] for r in sqlite3.connect(sys.argv[1]).execute(sys.argv[2])))" "$1" "$2"; }
TYPES_FILE=$(mktemp)
trap 'rm -f "$TYPES_FILE"' EXIT
python3 -c "import sqlite3,sys,json; json.dump([{'name':t,'kind':k} for t,k in sqlite3.connect(sys.argv[1]).execute('select Type, Kind from Types')], open(sys.argv[2],'w'))" "$3" "$TYPES_FILE"
export TYPES_FILE
export RESOURCES=$(q "$3" "select ResourceType from Resources")
export RELIGIONS=$(q "$3" "select ReligionType from Religions")
export BELIEFS=$(q "$3" "select BeliefType from Beliefs")
export PANTHEONS=$(q "$3" "select BeliefType from Beliefs where BeliefClassType='BELIEF_CLASS_PANTHEON'")
export WONDERS=$(q "$3" "select BuildingType from Buildings where IsWonder=1")
node "$(dirname "$0")/export_game.js" "$1" "$2"
