-- civ6-econ: replays a simulator plan (Civ6EconPlan.lua, written by tools/plan_lua.py) as the local player.
-- Each turn: player choices, unit orders, city production, purchases and worked plots are made to match the plan,
-- checked over several rounds (requests are applied asynchronously), then the turn is ended.
-- Everything that could not be matched is printed as "ECONPLAN <json>" lines in Lua.log.
include("Civ6EconPlan")

local PR = {}
local me = 0
local units = { builder = {}, settler = {} }   -- simulator id -> game unit id
local orders = { builder = {}, settler = {} }  -- simulator id -> {x, y, op, what, turn (when to act)}
local frames, rounds, phase, ended_at, end_tries = 0, 0, "idle", 0, 0
local ROUND_FRAMES, MAX_ROUNDS, END_WAIT, FORCE_WAIT = 8, 30, 240, 160

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

-- per turn: requests made for one thing; the game may refuse (e.g. policies outside a change window). The game
-- processes the last turn's growth and production some frames into a turn, so a request can fail only for a while.
local MAX_TRIES = 10
local tries = {}
local function again(key)
  tries[key] = (tries[key] or 0) + 1
  if tries[key] == MAX_TRIES + 1 then log("refused", { what = key }) end
  return tries[key] <= MAX_TRIES
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

-- where a planned unit is first: a settler's first plot on its way (next to the city that made it), else none
local function first_plot(kind, id)
  local o = orders[kind][id]
  if o and o.way and #o.way > 0 then return o.way[1][1], o.way[1][2] end
  if o and o.from then return o.from[1], o.from[2] end
end

local function unit_of(kind, id)
  local uid = units[kind][id]
  if uid == nil then
    -- a unit made after its order came: the free one nearest where the plan has it first, so that a settler made
    -- in another city does not walk the plan's way from the wrong end (11/01 run)
    local fx, fy = first_plot(kind, id)
    local u = find_new_unit(kind, fx, fy)
    if u == nil then return nil end
    if u == nil then return nil end
    units[kind][id] = u:GetID()
    log("unit_mapped_late", { unit = kind, id = id })
    return u
  end
  return player():GetUnits():FindID(uid)
end

-- ---------------------------------------------------------------- player choices
local function ensure_player(p)
  local pl = player()
  if p.research then
    local row = GameInfo.Technologies[p.research]
    local techs = pl:GetTechs()
    -- only a tech the game can research now: one whose prerequisites are missing becomes the research target and
    -- stalls all research (11/01 run: 5 techs at turn 66)
    if row and not techs:HasTech(row.Index) and techs:GetResearchingTech() ~= row.Index and techs:CanResearch(row.Index)
       and again("research " .. p.research) then
      UI.RequestPlayerOperation(me, PlayerOperations.RESEARCH, { [PlayerOperations.PARAM_TECH_TYPE] = row.Hash,
        [PlayerOperations.PARAM_INSERT_MODE] = PlayerOperations.VALUE_EXCLUSIVE })
      return false
    end
  end
  local culture = pl:GetCulture()
  if p.civic then
    local row = GameInfo.Civics[p.civic]
    if row and not culture:HasCivic(row.Index) and culture:GetProgressingCivic() ~= row.Index and culture:CanProgress(row.Index)
       and again("civic " .. p.civic) then
      UI.RequestPlayerOperation(me, PlayerOperations.PROGRESS_CIVIC, { [PlayerOperations.PARAM_CIVIC_TYPE] = row.Hash,
        [PlayerOperations.PARAM_INSERT_MODE] = PlayerOperations.VALUE_EXCLUSIVE })
      return false
    end
  end
  if p.government then
    local row = GameInfo.Governments[p.government]
    if row and culture:GetCurrentGovernment() ~= row.Index and again("government " .. p.government) then
      if not culture:RequestChangeGovernment(row.Hash) then log("government_failed", { government = p.government }) end
      return false
    end
  end
  -- Policies only once the government is the plan's: a change made under the old government's slots uses up the
  -- turn's one change (PolicyChangeMade) before the new slots exist (11/01 run, turn 28).
  local gov_row = p.government and GameInfo.Governments[p.government]
  if p.policies and (gov_row == nil or culture:GetCurrentGovernment() == gov_row.Index) then
    local n = culture:GetNumPolicySlots()
    local slot_type, in_slot, have, want = {}, {}, {}, {}
    for i = 0, n - 1 do
      slot_type[i] = GameInfo.GovernmentSlots[culture:GetSlotType(i)].GovernmentSlotType
      local idx = culture:GetSlotPolicy(i)
      if idx ~= nil and idx >= 0 then in_slot[i] = GameInfo.Policies[idx].PolicyType; have[in_slot[i]] = i end
    end
    -- only policies the game has unlocked: clearing a slot for one it has not unlocked empties the slot (11/01 run:
    -- Colonization before Early Empire left the economic slot empty)
    for _, pol in ipairs(p.policies) do
      if culture:IsPolicyUnlocked(GameInfo.Policies[pol].Hash) then want[pol] = true end
    end
    -- the plan's policies already slotted stay where they are: the engine cannot slot a policy that is active in the
    -- same request (GovernmentScreen.lua: Culture::CanSlotPolicy first checks !IsPolicyActive), so only the slots
    -- that change are cleared and only the new policies are added
    local taken, clear, add, missing = {}, {}, {}, {}
    for pol in pairs(want) do if have[pol] ~= nil then taken[have[pol]] = true end end
    for _, pol in ipairs(p.policies) do if want[pol] and have[pol] == nil then missing[#missing + 1] = pol end end
    local function place(pol, wildcard_ok)
      local t = GameInfo.Policies[pol].GovernmentSlotType
      for i = 0, n - 1 do
        if not taken[i] and (slot_type[i] == t or (wildcard_ok and slot_type[i] == "SLOT_WILDCARD")) then
          taken[i] = true; add[i] = GameInfo.Policies[pol].Hash; return true
        end
      end
    end
    local left = {}
    for _, pol in ipairs(missing) do if not place(pol, false) then left[#left + 1] = pol end end
    for _, pol in ipairs(left) do if not place(pol, true) then log("policy_no_slot", { policy = pol }) end end
    -- as GovernmentScreen.lua does, a slot that gets a policy is in the clear list too, empty or not (11/01 run: an
    -- add to an empty slot left out of the list was refused every turn)
    for i = 0, n - 1 do
      if add[i] or (in_slot[i] and not want[in_slot[i]]) then clear[#clear + 1] = i end
    end
    if (#clear > 0 or next(add) ~= nil) and again("policies") then
      local adds = {}
      for i, h in pairs(add) do adds[#adds + 1] = { i, GameInfo.Policies[h] and GameInfo.Policies[h].PolicyType or tostring(h) } end
      log("policy_request", { clear = clear, add = adds, want = p.policies })
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
    -- with the simulator's way, the plot it is at by the end of this turn; else the destination
    local tx, ty = o.x, o.y
    if o.way and #o.way > 0 then
      for _, w in ipairs(o.way) do
        if w[3] <= turn then tx, ty = w[1], w[2] end
      end
    end
    if u:GetMovesRemaining() > 0 and (u:GetX() ~= tx or u:GetY() ~= ty) then
      UnitManager.RequestOperation(u, UnitOperationTypes.MOVE_TO, { [UnitOperationTypes.PARAM_X] = tx,
        [UnitOperationTypes.PARAM_Y] = ty })
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
  -- UnitPanel.lua checks an improvement by its index and requests it by its hash
  local check_p = p
  if o.op == "improve" then
    check_p = { [UnitOperationTypes.PARAM_X] = o.x, [UnitOperationTypes.PARAM_Y] = o.y,
                [UnitOperationTypes.PARAM_IMPROVEMENT_TYPE] = GameInfo.Improvements[o.what].Index }
  end
  -- UnitPanel.lua requests a listed improvement without checking it again; the check by index answered "no" with
  -- no reason for improvements the game listed as allowed (11/01 run), so only other operations are checked
  if o.op ~= "improve" and not can(u, op, check_p) then
    -- the game's reasons and the improvements it allows here, asked as UnitPanel.lua does (results wanted: true)
    local valid, reasons = {}, {}
    pcall(function()
      local here = { [UnitOperationTypes.PARAM_X] = u:GetX(), [UnitOperationTypes.PARAM_Y] = u:GetY() }
      local _, res = UnitManager.CanStartOperation(u, UnitOperationTypes.BUILD_IMPROVEMENT, nil, here, true)
      for _, i in ipairs((res and res[UnitOperationResults.IMPROVEMENTS]) or {}) do
        local row = GameInfo.Improvements[i]; valid[#valid + 1] = row and row.ImprovementType or tostring(i)
      end
      local _, why = UnitManager.CanStartOperation(u, op, nil, check_p, true)
      for _, r in ipairs((why and why[UnitOperationResults.FAILURE_REASONS]) or {}) do reasons[#reasons + 1] = Locale.Lookup(r) end
    end)
    log("unit_op_refused", { unit = kind, id = id, op = o.op, what = o.what, at = { o.x, o.y }, moves = u:GetMovesRemaining(),
                             unit_at = { u:GetX(), u:GetY() }, valid = valid, reasons = reasons,
                             charges = u:GetBuildCharges(), type = unit_type(u) })
    return true  -- try again next round / next turn
  end
  if kind == "builder" then o.charges = o.charges or u:GetBuildCharges() end
  if o.requested == turn and not o.why_logged then
    -- requested earlier this turn and no charge used: the game's reasons, asked as UnitPanel.lua does
    local reasons, allowed = {}, nil
    pcall(function()
      local okb, why = UnitManager.CanStartOperation(u, op, nil, check_p, true)
      allowed = okb
      for _, r in ipairs((why and why[UnitOperationResults.FAILURE_REASONS]) or {}) do reasons[#reasons + 1] = Locale.Lookup(r) end
    end)
    log("unit_op_no_effect", { unit = kind, id = id, op = o.op, what = o.what, at = { o.x, o.y }, allowed = allowed,
                               reasons = reasons, moves = u:GetMovesRemaining(), charges = u:GetBuildCharges(),
                               activity = (u.GetActivityType ~= nil) and u:GetActivityType() or nil })
    o.why_logged = true
  end
  o.requested = turn
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
  if pc.tries > MAX_TRIES then
    if pc.tries == MAX_TRIES + 1 then log("production_failed", { city = { pc.x, pc.y }, item = pc.item, current = cur }) end
    return true
  end
  CityManager.RequestOperation(c, CityOperationTypes.BUILD, p)
  return false
end

-- Citizens are managed as the city panel does (CityPanel.lua, PlotInfo.lua): with the city selected and the
-- interface in CITY_MANAGEMENT mode, whose PARAM_MANAGE_CITIZEN parameter the MANAGE command needs (outside that mode
-- the parameter is empty and the command does nothing). The worked plots are read from the same command's targets.
-- logs what the citizen management reads in the first turns (a check of the API, read in Lua.log)
local PROBE_CITIZENS = false

local function with_city_screen(c, f)
  UI.SelectCity(c)
  UI.SetInterfaceMode(InterfaceModeTypes.CITY_MANAGEMENT)
  local param = UI.GetInterfaceModeParameter(CityCommandTypes.PARAM_MANAGE_CITIZEN)
  local ok, r = pcall(f, param)
  UI.SetInterfaceMode(InterfaceModeTypes.SELECTION)
  UI.DeselectAllCities()
  if not ok then log("citizens_error", { error = tostring(r) }); return true end
  return r
end

-- plot index -> citizens working it, and plot index -> those of them locked there; the city centre left out
local function worked_plots(c, param)
  local out, locked = {}, {}
  local t = CityManager.GetCommandTargets(c, CityCommandTypes.MANAGE, { [CityCommandTypes.PARAM_MANAGE_CITIZEN] = param })
  local plots, units, locks = t and t[CityCommandResults.PLOTS], t and t[CityCommandResults.CITIZENS], t and t[CityCommandResults.LOCKED_CITIZENS]
  for i, id in pairs(plots or {}) do
    local p = Map.GetPlotByIndex(id)
    if (units[i] or 0) > 0 and not (p:GetX() == c:GetX() and p:GetY() == c:GetY()) then
      out[id] = units[i]
      locked[id] = locks and locks[i] or 0
    end
  end
  return out, locked
end

-- A click on an unworked plot moves an unlocked citizen to it and locks it there; a click on a worked plot does not
-- take the citizen off (11/01 run: the plot stayed worked) but releases its lock. Citizens locked on plots the plan
-- does not work are released first, then the plan's plots are clicked; one click per round, then the plots are read
-- again.
local function ensure_citizens(c, pc)
  return with_city_screen(c, function(param)
    local have, locked = worked_plots(c, param)
    if PROBE_CITIZENS and Game.GetCurrentGameTurn() <= 5 then
      local h, plots = {}, {}
      for id, n in pairs(have) do local p = Map.GetPlotByIndex(id); h[#h + 1] = { p:GetX(), p:GetY(), n } end
      local cx, cy = c:GetX(), c:GetY()
      for dx = -1, 1 do for dy = -1, 1 do
        local p = Map.GetPlotXYWithRangeCheck(cx, cy, dx, dy, 1)
        if p then plots[#plots + 1] = { p:GetX(), p:GetY(), p:GetWorkerCount() } end
      end end
      local l = {}
      for id, n in pairs(locked) do local p = Map.GetPlotByIndex(id); l[#l + 1] = { p:GetX(), p:GetY(), n } end
      log("citizens_probe", { param = tostring(param), have = h, locked = l, plot_workers = plots,
                              food = c:GetYield(GameInfo.Yields["YIELD_FOOD"].Index) })
    end
    local want = {}
    for _, w in ipairs(pc.worked) do
      local pl = Map.GetPlot(w[1], w[2])
      if pl then want[pl:GetIndex()] = w[3] end
    end
    local function click(id, what)
      local pl = Map.GetPlotByIndex(id)
      if not again("citizens " .. pc.x .. "," .. pc.y .. " " .. what .. " " .. pl:GetX() .. "," .. pl:GetY()) then return true end
      CityManager.RequestCommand(c, CityCommandTypes.MANAGE, { [CityCommandTypes.PARAM_MANAGE_CITIZEN] = param,
        [CityCommandTypes.PARAM_X] = pl:GetX(), [CityCommandTypes.PARAM_Y] = pl:GetY() })
      return false
    end
    for id in pairs(have) do
      if want[id] == nil and (locked[id] or 0) > 0 then return click(id, "unlock") end
    end
    for id, n in pairs(want) do
      if (have[id] or 0) < n then return click(id, "on") end
    end
    return true
  end)
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

-- a plot bought by the city that owns its neighbourhood, as PlotInfo.lua does in the plot purchase mode
local function purchase_plot(b)
  local c = city_at(b.cx, b.cy)
  if c == nil then log("buy_no_city", b); return end
  local p = { [CityCommandTypes.PARAM_X] = b.x, [CityCommandTypes.PARAM_Y] = b.y,
              [CityCommandTypes.PARAM_PLOT_PURCHASE] = 1 }
  if not CityManager.CanStartCommand(c, CityCommandTypes.PURCHASE, p) then log("buy_plot_refused", b); return end
  CityManager.RequestCommand(c, CityCommandTypes.PURCHASE, p)
  log("buy", { plot = { b.x, b.y }, city = { b.cx, b.cy } })
end

-- ---------------------------------------------------------------- trade routes
-- orders the game may refuse for a while (no free trader yet, a governor title not yet earned) are kept this many turns
local KEEP_TURNS = 3
local routes = {}  -- the routes to start: {ox, oy, dx, dy, turn}, kept until one starts
local OP_TELEPORT = GameInfo.UnitOperations["UNITOPERATION_TELEPORT_TO_CITY"].Hash

local function route_params(dest, u)
  return { [UnitOperationTypes.PARAM_X0] = dest.dx, [UnitOperationTypes.PARAM_Y0] = dest.dy,
           [UnitOperationTypes.PARAM_X1] = u:GetX(), [UnitOperationTypes.PARAM_Y1] = u:GetY() }
end

local function check(u, op, p)
  local ok, r = pcall(function() return UnitManager.CanStartOperation(u, op, nil, p) end)
  return ok and r
end

-- A trader in the origin city starts the route (TradeRouteChooser.lua); a free trader elsewhere first moves to the
-- origin city as the trade origin chooser does (the simulator walks it there).
local function ensure_routes()
  local ok = true
  local claimed = {}  -- traders given an order in this round
  for i = #routes, 1, -1 do
    local r = routes[i]
    local started, moving = false, false
    for _, u in player():GetUnits():Members() do
      if unit_type(u) == "UNIT_TRADER" and not started and not moving and not claimed[u:GetID()] then
        if u:GetX() == r.ox and u:GetY() == r.oy and check(u, UnitOperationTypes.MAKE_TRADE_ROUTE, route_params(r, u)) then
          UnitManager.RequestOperation(u, UnitOperationTypes.MAKE_TRADE_ROUTE, route_params(r, u))
          log("route", { from = { r.ox, r.oy }, to = { r.dx, r.dy } })
          started, claimed[u:GetID()] = true, true
        end
      end
    end
    if not started then
      for _, u in player():GetUnits():Members() do
        local p = { [UnitOperationTypes.PARAM_X] = r.ox, [UnitOperationTypes.PARAM_Y] = r.oy }
        if unit_type(u) == "UNIT_TRADER" and not moving and not claimed[u:GetID()] and check(u, OP_TELEPORT, p) then
          UnitManager.RequestOperation(u, OP_TELEPORT, p)
          moving, claimed[u:GetID()] = true, true
        end
      end
    end
    if started then
      table.remove(routes, i)
    elseif Game.GetCurrentGameTurn() > r.turn + KEEP_TURNS then
      log("route_refused", r); table.remove(routes, i)
    elseif moving then
      ok = false
    end
  end
  return ok
end

-- ---------------------------------------------------------------- governors and dedications
local governors = {}  -- {op = "appoint", governor, x, y} / {op = "promote", governor, promotion}, plus turn

local function ensure_governors()
  local pg = player():GetGovernors()
  for i = #governors, 1, -1 do
    local g = governors[i]
    local row = GameInfo.Governors[g.governor]
    local done = false
    if g.op == "appoint" then
      local gov = pg:HasGovernor(row.Hash) and pg:GetGovernor(row.Hash) or nil
      local c = city_at(g.x, g.y)
      if gov == nil then
        if again("appoint " .. g.governor) then
          UI.RequestPlayerOperation(me, PlayerOperations.APPOINT_GOVERNOR, { [PlayerOperations.PARAM_GOVERNOR_TYPE] = row.Index })
        end
      elseif c == nil then
        log("governor_no_city", g); done = true
      elseif gov:GetAssignedCity() ~= nil and gov:GetAssignedCity():GetID() == c:GetID() then
        log("governor", g); done = true
      elseif again("assign " .. g.governor) then
        UI.RequestPlayerOperation(me, PlayerOperations.ASSIGN_GOVERNOR, { [PlayerOperations.PARAM_GOVERNOR_TYPE] = row.Index,
          [PlayerOperations.PARAM_PLAYER_ONE] = me, [PlayerOperations.PARAM_CITY_DEST] = c:GetID() })
      end
    else
      local prow = GameInfo.GovernorPromotions[g.promotion]
      local gov = pg:HasGovernor(row.Hash) and pg:GetGovernor(row.Hash) or nil
      if gov ~= nil and gov:HasPromotion(prow.Hash) then
        log("governor", g); done = true
      elseif gov ~= nil and again("promote " .. g.promotion) then
        UI.RequestPlayerOperation(me, PlayerOperations.PROMOTE_GOVERNOR, { [PlayerOperations.PARAM_GOVERNOR_TYPE] = row.Index,
          [PlayerOperations.PARAM_GOVERNOR_PROMOTION_TYPE] = prow.Index })
      end
    end
    if not done and Game.GetCurrentGameTurn() > g.turn + KEEP_TURNS then log("governor_refused", g); done = true end
    if done then table.remove(governors, i) end
  end
  return true
end

local function commemorate(c)
  UI.RequestPlayerOperation(me, PlayerOperations.COMMEMORATE,
    { [PlayerOperations.PARAM_COMMEMORATION_TYPE] = GameInfo.CommemorationTypes[c].Index })
  log("dedication", { what = c })
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
      orders.builder[b.id] = { x = b.x, y = b.y, op = b.op, what = b.what, turn = b.at or turn }
    end
  end
  for _, s in ipairs(d.settlers or {}) do
    if s.op == "go" then
      local fx, fy = s.x, s.y
      if s.way and #s.way > 0 then fx, fy = s.way[1][1], s.way[1][2] end
      local u = find_new_unit("settler", fx, fy)
      if u then units.settler[s.id] = u:GetID() else log("unit_not_found", { unit = "settler", id = s.id }) end
      orders.settler[s.id] = { x = s.x, y = s.y, op = "found", turn = s.arrive, way = s.way }
    end
  end
  if d.pantheon then pantheon_wanted = d.pantheon end
  for _, b in ipairs(d.buys or {}) do
    if b.kind == "item" then purchase(b) elseif b.kind == "tile" then purchase_plot(b) else log("unsupported_buy", b) end
  end
  for _, g in ipairs(d.governors or {}) do g.turn = turn; governors[#governors + 1] = g end
  for _, r in ipairs(d.routes or {}) do r.turn = turn; routes[#routes + 1] = r end
  if d.dedication then commemorate(d.dedication) end
  for _, u in ipairs(d.unsupported or {}) do log("unsupported", u) end
end

local function round(turn)
  local d = plan_of(turn)
  if d == nil then return true end
  if PROBE_CITIZENS and turn <= 8 then
    local c = player():GetCities():GetCapitalCity()
    if c then
      local q, item = c:GetBuildQueue(), current_item(c)
      local progress = nil
      if item and GameInfo.Units[item] then progress = q:GetUnitProgress(GameInfo.Units[item].Index)
      elseif item and GameInfo.Buildings[item] then progress = q:GetBuildingProgress(GameInfo.Buildings[item].Index)
      elseif item and GameInfo.Districts[item] then progress = q:GetDistrictProgress(GameInfo.Districts[item].Index) end
      log("turn_probe", { round = rounds, frames = frames, pop = c:GetPopulation(), food_store = c:GetGrowth():GetFood(),
                          item = item, progress = progress })
    end
  end
  local ok = ensure_player(d)
  ok = ensure_pantheon() and ok
  ok = ensure_units(turn) and ok
  ok = ensure_routes() and ok
  ok = ensure_governors() and ok
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
      with_city_screen(c, function(param)
        for idx, n in pairs(worked_plots(c, param)) do local pl = Map.GetPlotByIndex(idx); worked[#worked + 1] = { pl:GetX(), pl:GetY(), n } end
      end)
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
  elseif b == "ENDTURN_BLOCKING_COMMEMORATION_AVAILABLE" then
    -- the plan's dedication of the era: the simulator chooses it in a turn near the era's start
    for t = turn - 2, turn + 3 do
      local dt = plan_of(t)
      if dt and dt.dedication then commemorate(dt.dedication); return true end
    end
    return false
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
    elseif resolve(b, turn) and frames - ended_at <= FORCE_WAIT then
      if frames % (ROUND_FRAMES * 10) == 0 then log("resolving", { blocker = b }) end
    elseif frames - ended_at > 40 then
      -- a blocker the plan does not decide (envoys, governor titles the plan keeps, great people): the simulator
      -- does not act on it either, so the turn ends as Shift+Enter does (ActionPanel.lua)
      log("end_turn_forced", { blocker = b })
      phase, ended_at, end_tries = "ending", frames, end_tries + 1
      UI.RequestAction(ActionTypes.ACTION_ENDTURN, { REASON = "UserForced" })
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

-- Another player's diplomatic statement to us (a first meeting, a proposal) opens a screen that waits for the player.
-- The session is closed as DiplomacyActionView.lua does when diplomacy is not available: the plan does no diplomacy.
local function on_statement(from, to, k)
  if to == me and from ~= me then
    DiplomacyManager.CloseSession(k.SessionID)
    log("diplomacy_closed", { from = from })
  end
end

function PR.setup(on_finished, on_stuck)
  me = Game.GetLocalPlayer()
  Events.DiplomacyStatement.Add(on_statement)
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
