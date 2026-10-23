#!/bin/bash
# 10/24 France: the best play to T30 (food and production summed over the turns), with the districts and units the
# recorded player had by T30 required (3 campuses, a government plaza, 2 slingers, a trader); scouts (the simulator ties the recorded
# tribal villages and inspirations to them), settlers,
# builders and everything else free. Run from the repository root.
B=optimizer/target/release/civ6opt
G=data/games/france_20261024.json
C="--turn 30 --ext 0 --allow-military --team --require DISTRICT_CAMPUS:3@30,DISTRICT_GOVERNMENT:1@30,UNIT_SLINGER:2@30,UNIT_TRADER:1@30 --require-weight 2000"
D=data/search/france24
$B irollout $G $C --threads 16 --minutes 15 --window 30 --passes 20 --kick-to 28 --init record_search --seed 1 --out $D/ir1.json > $D/ir1.log 2>&1
