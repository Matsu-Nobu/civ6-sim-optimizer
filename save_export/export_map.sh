#!/bin/bash
# Usage: save_export/export_map.sh <save> <rules.sqlite> > map.json   (the ruleset's names for the parser, as export_game.sh)
set -e
q() { python3 -c "import sqlite3,sys; print(','.join(r[0] for r in sqlite3.connect(sys.argv[1]).execute(sys.argv[2])))" "$1" "$2"; }
TYPES_FILE=$(mktemp)
trap 'rm -f "$TYPES_FILE"' EXIT
python3 -c "import sqlite3,sys,json; json.dump([{'name':t,'kind':k} for t,k in sqlite3.connect(sys.argv[1]).execute('select Type, Kind from Types')], open(sys.argv[2],'w'))" "$2" "$TYPES_FILE"
export TYPES_FILE
export RESOURCES=$(q "$2" "select ResourceType from Resources")
node "$(dirname "$0")/export_map.js" "$1"
