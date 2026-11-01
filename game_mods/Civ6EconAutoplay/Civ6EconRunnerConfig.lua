-- Written by civ6-econ scripts. MODE: "off" (do nothing), "autoplay" (AI plays AUTOPLAY_TURNS),
-- "experiment" (load SAVE_NAME from the main menu, play TURNS turns, save each turn),
-- "plan" (load SAVE_NAME from the main menu, play Civ6EconPlan.lua as the local player, save each turn),
-- "newstart" (set up a new game like "newgame", found the capital, save EconPlanStart_<RUN_ID>_T002),
-- "newgame" (set up a new game with player 0 as LEADER, play TURNS turns, save each turn).
-- The setup of a new game is that of the user's multiplayer games (read from their saves' headers).
ECONRUN = { MODE = "off", SAVE_NAME = "EconPlanStart_mp1_T002", TURNS = 100, AUTOPLAY_TURNS = 150, RUN_ID = "mp1b",
            LEADER = "LEADER_TRAJAN", SPEED = "GAMESPEED_ONLINE", MAP = "Lakes.lua", MAP_SIZE = "MAPSIZE_HUGE",
            CITY_STATES = 18, NO_HUTS = false, BCY = 1, BCY_YIELD = 2 }
