-- civ6-econ: replays a simulator plan (Civ6EconPlan.lua, written by scripts/plan_export.py) as the local player.
-- Each turn: player choices, unit orders, city production, purchases and worked plots are made to match the plan,
-- checked over several rounds (requests are applied asynchronously), then the turn is ended.
-- Everything that could not be matched is printed as "ECONPLAN <json>" lines in Lua.log for scripts/plan_diff.py.
include("Civ6EconPlan")

local PR = {}
local me = 0
local units = { builder = {}, settler = {} }   -- simulator id -> game unit id
local orders = { builder = {}, settler = {} }  -- simulator id -> {x, y, op, what, turn (when to act)}
local frames, rounds, phase, ended_at, end_tries = 0, 0, "idle", 0, 0
local ROUND_FRAMES, MAX_ROUNDS, END_WAIT = 8, 30, 240

local function enc(v)
  local t = type(v)
  if t == "table" then
    local parts, n = {}, #v
    if n > 0 then
      for i = 1, n do parts[i] = enc(v[i]) end
      return "[" .. table.concat(parts, ",") .. "]"
    end
    for k, x in pairs(v) do parts[#parts + 1] = string.format("%q", tostring(k)) .. ":" .. enc(x) end
    return "{" .. table.concat(parts, ",") .. "}"
  elseif t == "string" then return string.format("%q", v)
  elseif t == "number" or t == "boolean" then return tostring(v) end
  return "null"
end

local function log(kind, fields)
  fields = fields or {}
  fields.kind = kind
  fields.turn = Game.GetCurrentGameTurn()
  print("ECONPLAN " .. enc(fields))
end

local function player() return Players[me] end

-- operations the UI issues by their database hash (UnitOperationTypes has no SKIP_TURN / SLEEP entries)
local OP_SKIP = GameInfo.UnitOperations["UNITOPERATION_SKIP_TURN"].Hash
local OP_SLEEP = GameInfo.UnitOperations["UNITOPERATION_SLEEP"].Hash

local tries = {}  -- per turn: requests made for one thing; the game may refuse (e.g. policies outside a change window)
local function again(key)
  tries[key] = (tries[key] or 0) + 1
  if tries[key] == 4 then log("refused", { what = key }) end
  return tries[key] <= 3
end

local function can(u, op, p)
  local ok, r = pcall(function() return UnitManager.CanStartOperation(u, op, nil, p) end)
  return not ok or r
end

local function city_at(x, y)
  for _, c in player():GetCities():Members() do
    if c:GetX() == x and c:GetY() == y then return c end
  end
end

local function unit_type(u) local row = GameInfo.Units[u:GetType()]; return row and row.UnitType end

local function find_new_unit(kind, x, y)
  local want = kind == "builder" and "UNIT_BUILDER" or "UNIT_SETTLER"
  local used = {}
  for _, id in pairs(units[kind]) do used[id] = true end
  local best, bestd
  for _, u in player():GetUnits():Members() do
    if unit_type(u) == want and not used[u:GetID()] then
      local d = Map.GetPlotDistance(u:GetX(), u:GetY(), x or u:GetX(), y or u:GetY())
      if best == nil or d < bestd or (d == bestd and u:GetID() < best:GetID()) then best, bestd = u, d end
    end
  end
  return best
end

local function unit_of(kind, id)
  local uid = units[kind][id]
  if uid == nil then
    local u = find_new_unit(kind, nil, nil)
    if u == nil then return nil end
    units[kind][id] = u:GetID()
    log("unit_mapped_late", { unit = kind, id = id })
    return u
  end
  return player():GetUnits():FindID(uid)
end

-- ---------------------------------------------------------------- player choices
local reslot = false
local function ensure_player(p)
  local pl = player()
  if p.research then
    local row = GameInfo.Technologies[p.research]
    local techs = pl:GetTechs()
    if row and not techs:HasTech(row.Index) and techs:GetResearchingTech() ~= row.Index and again("research " .. p.research) then
      UI.RequestPlayerOperation(me, PlayerOperations.RESEARCH, { [PlayerOperations.PARAM_TECH_TYPE] = row.Hash,
        [PlayerOperations.PARAM_INSERT_MODE] = PlayerOperations.VALUE_EXCLUSIVE })
      return false
    end
  end
  local culture = pl:GetCulture()
  if p.civic then
    local row = GameInfo.Civics[p.civic]
    if row and not culture:HasCivic(row.Index) and culture:GetProgressingCivic() ~= row.Index and again("civic " .. p.civic) then
      UI.RequestPlayerOperation(me, PlayerOperations.PROGRESS_CIVIC, { [PlayerOperations.PARAM_CIVIC_TYPE] = row.Hash,
        [PlayerOperations.PARAM_INSERT_MODE] = PlayerOperations.VALUE_EXCLUSIVE })
      return false
    end
  end
  if p.government then
    local row = GameInfo.Governments[p.government]
    if row and culture:GetCurrentGovernment() ~= row.Index and again("government " .. p.government) then
      if not culture:RequestChangeGovernment(row.Hash) then log("government_failed", { government = p.government }) end
      reslot = true  -- policies kept from the old government stay in their slots but do nothing until slotted again
      return false
    end
  end
  if p.policies then
    local n = culture:GetNumPolicySlots()
    local have, want = {}, {}
    for i = 0, n - 1 do
      local idx = culture:GetSlotPolicy(i)
      if idx ~= nil and idx >= 0 then have[GameInfo.Policies[idx].PolicyType] = true end
    end
    local same = true
    for _, pol in ipairs(p.policies) do want[pol] = true; if not have[pol] then same = false end end
    for pol in pairs(have) do if not want[pol] then same = false end end
    if (not same or reslot) and again("policies") then
      reslot = false
      local clear, add, taken = {}, {}, {}
      for i = 0, n - 1 do clear[#clear + 1] = i end
      local function place(pol, wildcard_ok)
        local slot_type = GameInfo.Policies[pol].GovernmentSlotType
        for i = 0, n - 1 do
          local st = GameInfo.GovernmentSlots[culture:GetSlotType(i)].GovernmentSlotType
          if not taken[i] and (st == slot_type or (wildcard_ok and st == "SLOT_WILDCARD")) then
            taken[i] = true; add[i] = GameInfo.Policies[pol].Hash; return true
          end
        end
      end
      local left = {}
      for _, pol in ipairs(p.policies) do if not place(pol, false) then left[#left + 1] = pol end end
      for _, pol in ipairs(left) do if not place(pol, true) then log("policy_no_slot", { policy = pol }) end end
      culture:RequestPolicyChanges(clear, add)
      return false
    end
  end
  return true
end

local pantheon_wanted = nil
local function ensure_pantheon()
  if pantheon_wanted == nil then return true end
  local rel = player():GetReligion()
  if rel:GetPantheon() >= 0 or not again("pantheon " .. pantheon_wanted) then return true end
  UI.RequestPlayerOperation(me, PlayerOperations.FOUND_PANTHEON, {
    [PlayerOperations.PARAM_BELIEF_TYPE] = GameInfo.Beliefs[pantheon_wanted].Hash,
    [PlayerOperations.PARAM_INSERT_MODE] = PlayerOperations.VALUE_EXCLUSIVE })
  return false
end

-- ---------------------------------------------------------------- units
local function act(kind, id, o, turn)
  local u = unit_of(kind, id)
  if u == nil then
    if not o.missing_logged then log("unit_missing", { unit = kind, id = id }); o.missing_logged = true end
    return true
  end
  if u:GetX() ~= o.x or u:GetY() ~= o.y then
    if u:GetMovesRemaining() > 0 then
      UnitManager.RequestOperation(u, UnitOperationTypes.MOVE_TO, { [UnitOperationTypes.PARAM_X] = o.x,
        [UnitOperationTypes.PARAM_Y] = o.y })
    end
    if o.op and turn >= o.turn and not o.late_logged then
      log("unit_late", { unit = kind, id = id, op = o.op, plan_turn = o.turn, at = { u:GetX(), u:GetY() }, to = { o.x, o.y } })
      o.late_logged = true
    end
    return true
  end
  if o.op == nil or turn < o.turn then return true end
  -- done once its effect shows: a charge used (builders) or the settler gone (city founded)
  if o.charges ~= nil and (u:GetBuildCharges() < o.charges) then
    log("unit_done", { unit = kind, id = id, op = o.op, plan_turn = o.turn })
    o.op, o.charges = nil, nil
    return true
  end
  if u:GetMovesRemaining() <= 0 then return true end
  if not again("unit " .. kind .. id) then return true end
  local p = { [UnitOperationTypes.PARAM_X] = o.x, [UnitOperationTypes.PARAM_Y] = o.y }
  local op
  if o.op == "found" then op = UnitOperationTypes.FOUND_CITY
  elseif o.op == "improve" then
    op = UnitOperationTypes.BUILD_IMPROVEMENT
    p[UnitOperationTypes.PARAM_IMPROVEMENT_TYPE] = GameInfo.Improvements[o.what].Hash
  elseif o.op == "harvest" and o.what == "resource" then op = UnitOperationTypes.HARVEST_RESOURCE
  elseif o.op == "harvest" then op = UnitOperationTypes.REMOVE_FEATURE
  elseif o.op == "plant" then op = GameInfo.UnitOperations["UNITOPERATION_PLANT_FOREST"].Hash
  end
  if op == nil then log("unsupported_unit_op", { unit = kind, id = id, op = o.op }); return true end
  if not can(u, op, p) then
    local valid = {}
    pcall(function()
      local okb, res = UnitManager.CanStartOperation(u, UnitOperationTypes.BUILD_IMPROVEMENT, nil, true)
      for _, i in ipairs((res and res[UnitOperationResults.IMPROVEMENTS]) or {}) do
        local row = GameInfo.Improvements[i]; valid[#valid + 1] = row and row.ImprovementType or tostring(i)
      end
    end)
    log("unit_op_refused", { unit = kind, id = id, op = o.op, what = o.what, at = { o.x, o.y }, moves = u:GetMovesRemaining(),
                             unit_at = { u:GetX(), u:GetY() }, valid = valid })
    return true  -- try again next round / next turn
  end
  if kind == "builder" then o.charges = o.charges or u:GetBuildCharges() end
  UnitManager.RequestOperation(u, op, p)
  log("unit_op", { unit = kind, id = id, op = o.op, what = o.what, at = { o.x, o.y }, plan_turn = o.turn })
  if kind == "settler" then o.op = nil end
  return false
end

local function ensure_units(turn)
  local ok = true
  for _, kind in ipairs({ "settler", "builder" }) do
    for id, o in pairs(orders[kind]) do
      if not act(kind, id, o, turn) then ok = false end
    end
  end
  -- everything else waits (the plan does not move them)
  for _, u in player():GetUnits():Members() do
    local t = unit_type(u)
    if t ~= "UNIT_BUILDER" and t ~= "UNIT_SETTLER" and u:GetMovesRemaining() > 0 then
      UnitManager.RequestOperation(u, OP_SLEEP)
    end
  end
  for kind, ids in pairs(units) do  -- planned units with nothing to do this turn
    for id, uid in pairs(ids) do
      local u = player():GetUnits():FindID(uid)
      local o = orders[kind][id]
      if u and u:GetMovesRemaining() > 0 and (o == nil or (u:GetX() == o.x and u:GetY() == o.y and o.op == nil)) then
        UnitManager.RequestOperation(u, OP_SKIP)
      end
    end
  end
  return ok
end

-- ---------------------------------------------------------------- cities
local function build_params(item, sx, sy)
  local p = { [CityOperationTypes.PARAM_INSERT_MODE] = CityOperationTypes.VALUE_EXCLUSIVE }
  if GameInfo.Units[item] then p[CityOperationTypes.PARAM_UNIT_TYPE] = GameInfo.Units[item].Hash
  elseif GameInfo.Districts[item] then p[CityOperationTypes.PARAM_DISTRICT_TYPE] = GameInfo.Districts[item].Hash
  elseif GameInfo.Buildings[item] then p[CityOperationTypes.PARAM_BUILDING_TYPE] = GameInfo.Buildings[item].Hash
  elseif GameInfo.Projects[item] then p[CityOperationTypes.PARAM_PROJECT_TYPE] = GameInfo.Projects[item].Hash
  else return nil end
  if sx then p[CityOperationTypes.PARAM_X] = sx; p[CityOperationTypes.PARAM_Y] = sy end
  return p
end

local function current_item(c)
  local h = c:GetBuildQueue():GetCurrentProductionTypeHash()
  if h == nil or h == 0 then return nil end
  for _, t in ipairs({ "Units", "Buildings", "Districts", "Projects" }) do
    local row = GameInfo[t][h]
    if row then return row.UnitType or row.BuildingType or row.DistrictType or row.ProjectType end
  end
  return tostring(h)
end

local function ensure_production(c, pc)
  if pc.item == nil then return true end
  local cur = current_item(c)
  if cur == pc.item then return true end
  local p = build_params(pc.item, pc.sx, pc.sy)
  if p == nil then log("unknown_item", { item = pc.item }); return true end
  pc.tries = (pc.tries or 0) + 1
  if pc.tries > 3 then
    if pc.tries == 4 then log("production_failed", { city = { pc.x, pc.y }, item = pc.item, current = cur }) end
    return true
  end
  CityManager.RequestOperation(c, CityOperationTypes.BUILD, p)
  return false
end

-- worked plots, read from the plots themselves (the MANAGE command targets come back empty outside the city screen)
local function worked_plots(c)
  local out = {}
  local cx, cy = c:GetX(), c:GetY()
  for dx = -3, 3 do
    for dy = -3, 3 do
      local p = Map.GetPlotXYWithRangeCheck(cx, cy, dx, dy, 3)
      if p and p:GetOwner() == me and Cities.GetPlotPurchaseCity(p) == c and p:GetWorkerCount() > 0
         and not (p:GetX() == cx and p:GetY() == cy) then
        out[p:GetIndex()] = p:GetWorkerCount()
      end
    end
  end
  return out
end

-- clicking an unworked plot moves a citizen there and locks it; one click per round, then look again
local function ensure_citizens(c, pc)
  local have = worked_plots(c)
  for _, w in ipairs(pc.worked) do
    local pl = Map.GetPlot(w[1], w[2])
    if pl and not (w[1] == pc.x and w[2] == pc.y) and (have[pl:GetIndex()] or 0) < w[3] then
      if not again("citizens " .. pc.x .. "," .. pc.y .. " " .. w[1] .. "," .. w[2]) then return true end
      CityManager.RequestCommand(c, CityCommandTypes.MANAGE, {
        [CityCommandTypes.PARAM_MANAGE_CITIZEN] = UI.GetInterfaceModeParameter(CityCommandTypes.PARAM_MANAGE_CITIZEN),
        [CityCommandTypes.PARAM_X] = w[1], [CityCommandTypes.PARAM_Y] = w[2] })
      return false
    end
  end
  return true
end

local function purchase(b)
  local c = city_at(b.x, b.y)
  if c == nil then log("buy_no_city", b); return end
  local p = build_params(b.item)
  p[CityOperationTypes.PARAM_INSERT_MODE] = nil
  local cp = {}
  for k, v in pairs(p) do cp[k] = v end
  cp[CityCommandTypes.PARAM_YIELD_TYPE] = GameInfo.Yields[b.yield].Index
  CityManager.RequestCommand(c, CityCommandTypes.PURCHASE, cp)
  log("buy", { item = b.item, city = { b.x, b.y }, yield = b.yield })
end

-- ---------------------------------------------------------------- the turn
local function plan_of(turn) return ECONPLAN.TURNS[turn] end

local function start_turn(turn)
  local d = plan_of(turn)
  if d == nil then return end
  for _, b in ipairs(d.builders or {}) do
    if b.op == "new" then
      local u = find_new_unit("builder", b.x, b.y)
      if u then units.builder[b.id] = u:GetID() else log("unit_not_found", { unit = "builder", id = b.id }) end
    elseif b.op == "go" then
      orders.builder[b.id] = { x = b.x, y = b.y }
    else
      orders.builder[b.id] = { x = b.x, y = b.y, op = b.op, what = b.what, turn = turn }
    end
  end
  for _, s in ipairs(d.settlers or {}) do
    if s.op == "go" then
      local u = find_new_unit("settler", nil, nil)
      if u then units.settler[s.id] = u:GetID() else log("unit_not_found", { unit = "settler", id = s.id }) end
      orders.settler[s.id] = { x = s.x, y = s.y, op = "found", turn = s.arrive }
    end
  end
  if d.pantheon then pantheon_wanted = d.pantheon end
  for _, b in ipairs(d.buys or {}) do
    if b.kind == "item" then purchase(b) else log("unsupported_buy", b) end
  end
  for _, g in ipairs(d.governors or {}) do log("unsupported_governor", { what = g }) end
  for _, r in ipairs(d.routes or {}) do log("unsupported_route", r) end
end

local function round(turn)
  local d = plan_of(turn)
  if d == nil then return true end
  local ok = ensure_player(d)
  ok = ensure_pantheon() and ok
  ok = ensure_units(turn) and ok
  for _, pc in ipairs(d.cities or {}) do
    local c = city_at(pc.x, pc.y)
    if c == nil then
      if rounds == MAX_ROUNDS - 1 then log("city_missing", { city = { pc.x, pc.y } }) end
      ok = false
    else
      ok = ensure_production(c, pc) and ok
      ok = ensure_citizens(c, pc) and ok
    end
  end
  return ok
end

local function report(turn)
  local d = plan_of(turn)
  if d == nil then return end
  for _, pc in ipairs(d.cities or {}) do
    local c = city_at(pc.x, pc.y)
    if c then
      local worked = {}
      for idx, n in pairs(worked_plots(c)) do local pl = Map.GetPlotByIndex(idx); worked[#worked + 1] = { pl:GetX(), pl:GetY(), n } end
      local y = {}
      for row in GameInfo.Yields() do y[row.YieldType] = c:GetYield(row.Index) end
      local g = c:GetGrowth()
      log("city", { city = { pc.x, pc.y }, item = current_item(c), worked = worked, yields = y,
                    pop = c:GetPopulation(), food_surplus = g:GetFoodSurplus(), food_store = g:GetFood() })
    end
  end
  local pl = player()
  local t, cv = pl:GetTechs():GetResearchingTech(), pl:GetCulture():GetProgressingCivic()
  log("player", { research = t >= 0 and GameInfo.Technologies[t].TechnologyType or nil,
                  civic = cv >= 0 and GameInfo.Civics[cv].CivicType or nil })
end

local function blocker()
  local id = NotificationManager.GetFirstEndTurnBlocking(me)
  for k, v in pairs(EndTurnBlockingTypes) do if v == id then return k end end
  return tostring(id)
end

local function yields_key()
  local parts = {}
  for _, c in player():GetCities():Members() do
    for row in GameInfo.Yields() do parts[#parts + 1] = tostring(c:GetYield(row.Index)) end
    parts[#parts + 1] = tostring(c:GetPopulation())
  end
  return table.concat(parts, ",")
end

-- Ending a turn: the human player's cities are processed (growth, production) when the turn is ended, even if the
-- end is then held by a blocker; a held request also lingers into the next turn. So the turn is ended only once,
-- with nothing blocking, after the plan's state has held still (worked plots and yields) for a while.
local function idle_units()
  for _, u in player():GetUnits():Members() do
    if u:GetMovesRemaining() > 0 then UnitManager.RequestOperation(u, OP_SKIP) end
  end
end

-- Decisions due when the turn is ended (research or production finished during the end-of-turn processing) are the
-- ones the plan makes at the start of the next turn.
local function resolve(b, turn)
  local d = plan_of(turn + 1) or {}
  if b == "ENDTURN_BLOCKING_RESEARCH" then ensure_player({ research = d.research })
  elseif b == "ENDTURN_BLOCKING_CIVIC" then ensure_player({ civic = d.civic })
  elseif b == "ENDTURN_BLOCKING_FILL_CIVIC_SLOT" then ensure_player({ policies = d.policies })
  elseif b == "ENDTURN_BLOCKING_CONSIDER_GOVERNMENT_CHANGE" then
    ensure_player({ government = d.government })
    pcall(function() player():GetCulture():SetGovernmentChangeConsidered(true) end)
  elseif b == "ENDTURN_BLOCKING_PRODUCTION" then
    for _, pc in ipairs(d.cities or {}) do
      local c = city_at(pc.x, pc.y)
      if c and current_item(c) == nil then ensure_production(c, pc) end
    end
  elseif b == "ENDTURN_BLOCKING_PANTHEON" then
    for t = turn + 1, ECONPLAN.LAST do
      local dt = plan_of(t)
      if dt and dt.pantheon then pantheon_wanted = dt.pantheon; break end
    end
    ensure_pantheon()
  else
    return false
  end
  return true
end

local settle_key, settle_count, unblock_tried = nil, 0, false
local function on_update()
  frames = frames + 1
  if frames == 1 then log("tick", {}) end
  local turn = Game.GetCurrentGameTurn()
  if phase == "acting" then
    if frames % ROUND_FRAMES ~= 0 then return end
    rounds = rounds + 1
    local ok = round(turn)
    local key = yields_key()
    if ok and key == settle_key then settle_count = settle_count + 1 else settle_count = 0 end
    settle_key = key
    if (ok and settle_count >= 3) or rounds >= MAX_ROUNDS then
      if not ok then log("round_limit", {}) end
      report(turn)
      phase, ended_at, unblock_tried = "unblocking", frames, false
    end
  elseif phase == "unblocking" then
    if frames % ROUND_FRAMES ~= 0 then return end
    local b = blocker()
    if b == "NO_ENDTURN_BLOCKING" then
      phase, ended_at, end_tries = "ending", frames, end_tries + 1
      if end_tries == 1 then
        UI.RequestAction(ActionTypes.ACTION_ENDTURN)
      else  -- a plain end was ignored with nothing blocking (units with moves left): end as Shift+Enter does
        log("end_turn_forced", { blocker = b })
        UI.RequestAction(ActionTypes.ACTION_ENDTURN, { REASON = "UserForced" })
      end
    elseif b == "ENDTURN_BLOCKING_UNITS" or b == "ENDTURN_BLOCKING_UNIT_NEEDS_ORDERS" or b == "ENDTURN_BLOCKING_STACKED_UNITS" then
      -- units only: tell them to wait once; if that does not clear it, end as Shift+Enter does (units may keep moves)
      if not unblock_tried then
        unblock_tried = true
        idle_units()
      elseif frames - ended_at > 40 then
        log("end_turn_forced", { blocker = b })
        phase, ended_at, end_tries = "ending", frames, end_tries + 1
        UI.RequestAction(ActionTypes.ACTION_ENDTURN, { REASON = "UserForced" })
      end
    elseif resolve(b, turn) then
      if frames % (ROUND_FRAMES * 10) == 0 then log("resolving", { blocker = b }) end
    end
    if phase == "unblocking" and frames - ended_at > END_WAIT then
      log("end_turn_blocked", { blocker = b })
      PR.on_stuck(); phase = "idle"; ContextPtr:ClearUpdate()
    end
  elseif phase == "ending" and frames - ended_at > END_WAIT then
    -- the turn ends only after every other player has moved: once the request is in, just wait for the next turn
    -- (asking again would carry over and end the next turn before the plan acts in it)
    local sent = false
    pcall(function() sent = UI.HasSentTurnComplete() end)
    if sent then ended_at = frames; return end
    log("end_turn_ignored", { blocker = blocker(), tries = end_tries })
    if end_tries >= 3 then PR.on_stuck(); phase = "idle"; ContextPtr:ClearUpdate(); return end
    phase, ended_at, unblock_tried = "unblocking", frames, false
  end
end

function PR.begin_turn()
  local turn = Game.GetCurrentGameTurn()
  if PR.last_begun == turn then return end
  PR.last_begun = turn
  if turn > ECONPLAN.LAST then PR.on_finished(); return end
  if not Players[me]:IsHuman() then log("not_human", {}) end
  tries = {}
  start_turn(turn)
  frames, rounds, phase, end_tries = 0, 0, "acting", 0
  ContextPtr:SetHide(false)  -- a hidden context gets no update calls (the context has no controls, nothing shows)
  ContextPtr:SetUpdate(on_update)
end

function PR.setup(on_finished, on_stuck)
  me = Game.GetLocalPlayer()
  PR.on_finished, PR.on_stuck = on_finished, on_stuck
  -- builders/settlers already on the map at the start carry no simulator id (the plan starts with none)
  -- a save made during autoplay keeps the AI playing the local player: stop that
  local ok, err = pcall(function()
    AutoplayManager.SetReturnAsPlayer(me)
    AutoplayManager.SetObserveAsPlayer(me)
    AutoplayManager.SetActive(false)
  end)
  log("plan_start", { start = ECONPLAN.START, last = ECONPLAN.LAST, me = me, human = Players[me]:IsHuman(),
                      autoplay_off = ok and true or tostring(err) })
end

ECONPLANRUNNER = PR
