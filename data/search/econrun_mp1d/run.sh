#!/bin/bash
# 11/02: a plan to play in the game from EconPlanStart_mp1_T002 that chops, builds lumber mills and districts (the
# rules the in-game check has not met yet): an online search with those as requirements, guided by econrun_mp1c's plan.
B=optimizer/target/release/civ6opt
G=data/games/econrun_mp1.json
D=data/search/econrun_mp1d
O="--turn 65 --ext 20 --ext-model --fallback-model data/search/econ21f/m1.json --require harvests:8@45,IMPROVEMENT_LUMBER_MILL:4@60,DISTRICT_CAMPUS:2@60,DISTRICT_COMMERCIAL_HUB:2@60 --require-weight 3000"
[ -f $D/online2.json ] || $B online $G $O --guide data/search/econrun_mp1c/online1.json --top 8 --threads 14 --out $D/online2.json > /dev/null 2> $D/online2.err
$B run $G $O --genome $D/online2.json 2>/dev/null | tail -1 > $D/online2.score
