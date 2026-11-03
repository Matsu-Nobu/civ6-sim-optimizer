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

-- the plan runner's resume saves (Civ6EconPlanRunner.lua)
ECONRUN_SAVE_NAMED = save_named

local function save_turn()
  -- "newstart" writes only its start save
  if ECONRUN.MODE == "off" or ECONRUN.MODE == "newstart" then return end
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

-- The game is not closed from here: closing it this way crashed the game. The user closes it.
local function quit()
  print("ECONRUN done; the game can be closed")
  ContextPtr:ClearUpdate()
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

-- MODE "probe": the cities of player ECONRUN.PROBE[i].player as the game computes them right after loading the save
-- (worked plots' yields, city yields, food surplus, amenities, the item and its production), as ECONPROBE lines in
-- Lua.log; then the next save of ECONRUN.PROBE is loaded (the saves are in the single-player folder).
local function json(v)
  local t = type(v)
  if t == "table" then
    local parts, n = {}, #v
    if n > 0 then
      for _, x in ipairs(v) do parts[#parts + 1] = json(x) end
      return "[" .. table.concat(parts, ",") .. "]"
    end
    for k, x in pairs(v) do parts[#parts + 1] = string.format("%q:%s", tostring(k), json(x)) end
    return "{" .. table.concat(parts, ",") .. "}"
  elseif t == "string" then return string.format("%q", v)
  elseif t == "nil" then return "null"
  else return tostring(v) end
end

-- Writes a probe record. The game's log cuts a printed line at about 2000 bytes, so a long record goes out in pieces
-- of at most 1500 bytes ("ECONPART"), the last one as "ECONPROBE"; a piece never ends inside a UTF-8 character.
local function emit(t)
  local s = json(t)
  while #s > 1500 do
    local k = 1500
    while k > 1 and s:byte(k + 1) ~= nil and s:byte(k + 1) >= 0x80 and s:byte(k + 1) < 0xC0 do k = k - 1 end
    print("ECONPART " .. s:sub(1, k))
    s = s:sub(k + 1)
  end
  print("ECONPROBE " .. s)
end

local function probe()
  local turn = Game.GetCurrentGameTurn()
  local idx = nil
  for i, p in ipairs(ECONRUN.PROBE) do
    if p.game_turn == turn and Players[p.player] ~= nil and PlayerConfigurations[p.player]:GetCivilizationTypeName() == p.civ then idx = i end
  end
  emit({ kind = "loaded", turn = turn, index = idx })
  if idx ~= nil then
    local pid = ECONRUN.PROBE[idx].player
    -- the player's research and civics: cost and progress of each
    local ok5, err5 = pcall(function()
      local pl, techs, civics = Players[pid], {}, {}
      for row in GameInfo.Technologies() do
        techs[row.TechnologyType] = { pl:GetTechs():GetResearchCost(row.Index), pl:GetTechs():GetResearchProgress(row.Index) }
      end
      for row in GameInfo.Civics() do
        civics[row.CivicType] = { pl:GetCulture():GetCultureCost(row.Index), pl:GetCulture():GetCulturalProgress(row.Index) }
      end
      emit({ kind = "player", turn = turn, game = ECONRUN.PROBE[idx].game, techs = techs, civics = civics })
    end)
    if not ok5 then emit({ kind = "error", err = tostring(err5) }) end
    -- the boost experiment's civics (Civ6EconBoostTest.lua): every major player's civic costs and progress
    if ECONRUN.BOOSTS then
      for _, other in ipairs(PlayerManager.GetAliveMajorIDs()) do
        local civics, cu = {}, Players[other]:GetCulture()
        for row in GameInfo.Civics() do
          civics[row.CivicType] = { cu:GetCultureCost(row.Index), cu:GetCulturalProgress(row.Index) }
        end
        emit({ kind = "civics", turn = turn, game = ECONRUN.PROBE[idx].game, player = other, civics = civics })
      end
    end
    local yields = {}
    for row in GameInfo.Yields() do yields[#yields + 1] = row.YieldType end
    for _, city in Players[pid]:GetCities():Members() do
      local ok, err = pcall(function()
        local plots = {}
        for _, i in ipairs(Map.GetCityPlots():GetPurchasedPlots(city)) do
          local p = Map.GetPlotByIndex(i)
          if p:GetWorkerCount() > 0 or (p:GetX() == city:GetX() and p:GetY() == city:GetY()) then
            local y = {}
            for k = 0, #yields - 1 do y[#y + 1] = p:GetYield(k) end
            plots[#plots + 1] = { x = p:GetX(), y = p:GetY(), workers = p:GetWorkerCount(), yields = y }
          end
        end
        local cy, tips = {}, {}
        for k = 0, #yields - 1 do
          cy[#cy + 1] = city:GetYield(k)
          local ok2, tip = pcall(function() return city:GetYieldToolTip(k) end)
          tips[#tips + 1] = ok2 and (tostring(tip):gsub("[\r\n]", "|"):gsub("\\", "/")) or ""
        end
        local g, q = city:GetGrowth(), city:GetBuildQueue()
        -- the costs and progress of the units the city could make, and of its current item
        local costs = {}
        for _, u in ipairs({ "UNIT_BUILDER", "UNIT_SETTLER", "UNIT_TRADER", "UNIT_HORSEMAN", "UNIT_HEAVY_CHARIOT" }) do
          local row = GameInfo.Units[u]
          if row ~= nil then
            local ok3, c = pcall(function() return q:GetUnitCost(row.Index) end)
            local ok4, p = pcall(function() return q:GetUnitProgress(row.Index) end)
            costs[u] = { ok3 and c or nil, ok4 and p or nil }
          end
        end
        -- (a method this game version lacks gives null)
        local function try(o, m) local ok, v = pcall(function() return o[m](o) end); if ok then return v end; return nil end
        emit({ kind = "city", turn = turn, game = ECONRUN.PROBE[idx].game, name = city:GetName(),
          pop = city:GetPopulation(), yields = cy, food_surplus = try(g, "GetFoodSurplus"), amenities = try(g, "GetAmenities"),
          amenities_needed = try(g, "GetAmenitiesNeeded"), happiness = try(g, "GetHappiness"), item = try(q, "CurrentlyBuilding"),
          production = try(q, "GetProductionYield"), growth_threshold = try(g, "GetGrowthThreshold"), food = try(g, "GetFood"),
          plots = plots, tips = tips, costs = costs, held = try(q, "GetOverflow") })
      end)
      if not ok then emit({ kind = "error", err = tostring(err) }) end
    end
  end
  local nxt = ECONRUN.PROBE[(idx or 0) + 1]
  if nxt == nil then emit({ kind = "done" }); return end
  local g = {}
  g.Location = SaveLocations.LOCAL_STORAGE
  g.Type = SaveTypes.SINGLE_PLAYER
  g.IsAutosave = false
  g.IsQuicksave = false
  g.Directory = SaveDirectories.DEFAULT
  g.Name = nxt.name
  emit({ kind = "next", name = nxt.name })
  Network.LoadGame(g, ServerType.SERVER_TYPE_NONE)
end

local function start()
  if started or ECONRUN.MODE == "off" then return end
  started = true
  if ECONRUN.MODE == "probe" then
    local ok, err = pcall(probe)
    if not ok then emit({ kind = "error", err = tostring(err) }) end
    return
  end
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
Events.TurnBegin.Add(function() if ECONRUN.MODE ~= "off" and ECONRUN.MODE ~= "probe" then save_turn() end end)  -- a save every turn
LuaEvents.AutoPlayEnd.Add(on_autoplay_end)  -- raised by the engine AutoPlay_Manager
