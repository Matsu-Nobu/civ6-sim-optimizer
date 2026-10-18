-- civ6-econ runner, in game. Behaviour set by Civ6EconRunnerConfig.lua (MODE "off" does nothing).
include("Civ6EconRunnerConfig")
if ECONRUN.MODE == "plan" then include("Civ6EconPlanRunner") end
local started = false
local tag = nil

local function save_named(name)
  local s = {}
  s.Name = name
  s.Location = SaveLocations.LOCAL_STORAGE
  s.Type = SaveTypes.SINGLE_PLAYER
  s.IsAutosave = false
  s.IsQuicksave = false
  Network.SaveGame(s)
end

local function save_turn()
  if ECONRUN.MODE == "off" then return end
  local ok, err = pcall(function()
    local turn = Game.GetCurrentGameTurn()
    if ECONRUN.MODE == "experiment" or ECONRUN.MODE == "newgame" or ECONRUN.MODE == "plan" then
      save_named(string.format("EconRun_%s_T%03d", ECONRUN.RUN_ID, turn))
    else
      if tag == nil then tag = tostring(math.floor((Game.GetRandNum and Game.GetRandNum(100000, "EconAuto") or 0))) end
      save_named(string.format("EconAuto_%s_T%03d", tag, turn))
    end
  end)
  if not ok then print("ECONRUN save failed: " .. tostring(err)) end
end

local function quit()
  print("ECONRUN done, quitting")
  Events.UserConfirmedClose()
end

local function on_autoplay_end()
  if ECONRUN.MODE ~= "experiment" and ECONRUN.MODE ~= "newgame" then return end
  print("ECONRUN autoplay ended at turn " .. Game.GetCurrentGameTurn())
  save_turn()
  -- saves are not immediate; quit once it is written
  Events.SaveComplete.Add(function() quit() end)
end

local function finish_and_quit(why)
  print("ECONRUN plan " .. why .. " at turn " .. Game.GetCurrentGameTurn())
  Events.SaveComplete.Add(function() quit() end)
  save_turn()
end

-- MODE "newstart": a new game played by the local player (never autoplayed, so it stays human) up to turn 2:
-- found the capital where the settler stands, first build a Scout, then save EconPlanStart_<RUN_ID>_T002 and quit.
local function newstart()
  local me, frames, saving = Game.GetLocalPlayer(), 0, false
  ContextPtr:SetHide(false)
  ContextPtr:SetUpdate(function()
    frames = frames + 1
    if frames % 10 ~= 0 or saving then return end
    local ok, err = pcall(function()
      local pl = Players[me]
      if Game.GetCurrentGameTurn() >= 2 then
        saving = true
        Events.SaveComplete.Add(function() quit() end)
        save_named(string.format("EconPlanStart_%s_T002", ECONRUN.RUN_ID))
        print("ECONRUN newstart saved, human " .. tostring(pl:IsHuman()))
        return
      end
      for _, u in pl:GetUnits():Members() do
        if u:GetMovesRemaining() > 0 then
          if GameInfo.Units[u:GetType()].UnitType == "UNIT_SETTLER" then
            UnitManager.RequestOperation(u, UnitOperationTypes.FOUND_CITY)
          else
            UnitManager.RequestOperation(u, GameInfo.UnitOperations["UNITOPERATION_SKIP_TURN"].Hash)
          end
        end
      end
      for _, c in pl:GetCities():Members() do
        local h = c:GetBuildQueue():GetCurrentProductionTypeHash()
        if h == nil or h == 0 then
          CityManager.RequestOperation(c, CityOperationTypes.BUILD, {
            [CityOperationTypes.PARAM_UNIT_TYPE] = GameInfo.Units["UNIT_SCOUT"].Hash,
            [CityOperationTypes.PARAM_INSERT_MODE] = CityOperationTypes.VALUE_EXCLUSIVE })
        end
      end
      if pl:GetTechs():GetResearchingTech() < 0 then
        UI.RequestPlayerOperation(me, PlayerOperations.RESEARCH, {
          [PlayerOperations.PARAM_TECH_TYPE] = GameInfo.Technologies["TECH_POTTERY"].Hash,
          [PlayerOperations.PARAM_INSERT_MODE] = PlayerOperations.VALUE_EXCLUSIVE })
      end
      if pl:GetCulture():GetProgressingCivic() < 0 then
        UI.RequestPlayerOperation(me, PlayerOperations.PROGRESS_CIVIC, {
          [PlayerOperations.PARAM_CIVIC_TYPE] = GameInfo.Civics["CIVIC_CODE_OF_LAWS"].Hash,
          [PlayerOperations.PARAM_INSERT_MODE] = PlayerOperations.VALUE_EXCLUSIVE })
      end
      if frames % 60 == 0 and pl:GetCities():GetCount() > 0 then UI.RequestAction(ActionTypes.ACTION_ENDTURN) end
    end)
    if not ok then print("ECONRUN newstart error " .. tostring(err)) end
  end)
end

local function start()
  if started or ECONRUN.MODE == "off" then return end
  started = true
  if ECONRUN.MODE == "newstart" then
    print("ECONRUN start mode newstart, human " .. tostring(Players[Game.GetLocalPlayer()]:IsHuman()))
    newstart()
    return
  end
  if ECONRUN.MODE == "plan" then
    ECONPLANRUNNER.setup(function() finish_and_quit("finished") end, function() finish_and_quit("stuck") end)
    Events.LocalPlayerTurnBegin.Add(function() ECONPLANRUNNER.begin_turn() end)
    ECONPLANRUNNER.begin_turn()
    print("ECONRUN start mode plan turn " .. Game.GetCurrentGameTurn())
    return
  end
  local turns = (ECONRUN.MODE == "experiment" or ECONRUN.MODE == "newgame") and ECONRUN.TURNS or ECONRUN.AUTOPLAY_TURNS
  local ok, err = pcall(function()
    AutoplayManager.SetTurns(turns)
    AutoplayManager.SetReturnAsPlayer(0)
    AutoplayManager.SetObserveAsPlayer(0)
    AutoplayManager.SetActive(true)
  end)
  print("ECONRUN start mode " .. ECONRUN.MODE .. " turn " .. Game.GetCurrentGameTurn() .. " " ..
        (ok and ("autoplay " .. turns .. " turns") or ("failed: " .. tostring(err))))
  if ECONRUN.MODE == "autoplay" then save_turn() end
end

Events.LoadScreenClose.Add(start)
Events.TurnBegin.Add(function() if ECONRUN.MODE ~= "off" then save_turn() end end)  -- a save every turn
LuaEvents.AutoPlayEnd.Add(on_autoplay_end)  -- raised by the engine AutoPlay_Manager
