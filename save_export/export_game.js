// Export one player's per-turn state from a folder of saves as JSON Lines, for the Python simulator.
// Usage: node save_export/export_game.js <saveDir> <playerId> > out.jsonl
const { parseCiv6Save, decompressCiv6Payload } = require('../third_party/civ6-savegame-parser');
const { civ6Hash, registerTypes } = require('../third_party/civ6-savegame-parser/dist/internal/hash-tables');
// Teach the parser the ruleset's own type names (mod additions such as BBG governor promotions).
const WONDERS = new Set((process.env.WONDERS ?? '').split(',').filter(Boolean));
if (process.env.TYPES_FILE) registerTypes(JSON.parse(require('fs').readFileSync(process.env.TYPES_FILE, 'utf8')));
const { resolveTypeHash } = require('../third_party/civ6-savegame-parser/dist/internal/hash-tables');

// District records `15, instanceId, x, y, cityId, DISTRICT_* hash, damage, wallsDamage, lastDamagedTurn, 0, cost,
// built` (parser's layout). The parser only accepts instance ids shaped (n+1)<<16|n, which holds for city centres
// but not for other districts (a campus carried 262146 = 4<<16|2), so they were dropped. Here a record is
// accepted when its id and coordinates match a tile's district id.
function districtRecords(buf, plotDistrictId) {
  const out = new Map();
  for (let o = 0; o + 48 <= buf.length; o++) {
    if (buf.readInt32LE(o) !== 15) continue;
    const x = buf.readInt32LE(o + 8), y = buf.readInt32LE(o + 12);
    const key = `${x},${y}`;
    const id = buf.readUInt32LE(o + 4);
    if (plotDistrictId.get(key) !== id || out.has(key)) continue;
    const t = resolveTypeHash(buf.readUInt32LE(o + 20));
    if (!t || t.kind !== 'KIND_DISTRICT') continue;
    out.set(key, { type: t.name, complete: buf.readInt32LE(o + 44) === 1, cost: buf.readInt32LE(o + 40) });
  }
  return out;
}

// Per-city yields as CivVIReplay reads them (src/CivVIParser.cpp): `cityId, 0x11, ...` then two maps
// `6, (yieldHash, value*256) x6`. The first is after percentage modifiers, the second before.
// They hold the state of this turn (not the previous one). The FOOD entry is not the city's food.
// The runs of ids 0,1,2,... in the payload belong to the players with cities, in player id order.
const YIELD_NAMES = ['YIELD_FOOD', 'YIELD_PRODUCTION', 'YIELD_GOLD', 'YIELD_SCIENCE', 'YIELD_CULTURE', 'YIELD_FAITH'];
const YIELD_BY_HASH = new Map(YIELD_NAMES.map((y) => [civ6Hash(y) >>> 0, y]));
function cityYieldBlocks(buf) {
  const readMap = (i) => {
    if (buf.readUInt32LE(i) !== 6) return null;
    const m = {};
    for (let k = 0; k < 6; k++) {
      const y = YIELD_BY_HASH.get(buf.readUInt32LE(i + 4 + k * 8));
      if (!y) return null;
      m[y] = buf.readInt32LE(i + 8 + k * 8) / 256;
    }
    return m;
  };
  const runs = [];
  for (let i = 28; i + 110 < buf.length; i++) {
    if (buf.readUInt32LE(i) !== 6) continue;
    const tag = buf.readUInt32LE(i - 24);
    if (tag !== 0x10 && tag !== 0x11) continue;
    const a = readMap(i), b = a && readMap(i + 52);
    if (!b) continue;
    const id = buf.readUInt32LE(i - 28);
    if (id === 0) runs.push([]);
    if (runs.length && id === runs[runs.length - 1].length) runs[runs.length - 1].push({ modified: a, base: b });
    i += 103;
  }
  return runs;
}
// Faith and pantheon: `20, faith*256, 0, BELIEF_*|-1, n, x, y` (parser README). The parser misses it on
// these saves; the first such record in the payload is player 0's.
// The records follow the players in id order (with other records between): matched to the players one by one by
// faith (which the parser reads for every player), so equal faiths of two players do not mix them up.
function playerReligion(buf, pantheonHashes, players, pid) {
  let k = 0;
  for (let i = 0; i + 28 < buf.length && k < players.length; i += 1) {
    if (buf.readInt32LE(i) !== 20 || buf.readInt32LE(i + 8) !== 0) continue;
    const b = buf.readInt32LE(i + 12) >>> 0;
    if (b !== 0xffffffff && !pantheonHashes.has(b)) continue;
    const faith = buf.readInt32LE(i + 4);
    if (faith < 0 || faith > 1e6 * 256) continue;
    if (players[k].faith != null && Math.abs(faith / 256 - players[k].faith) > 1e-6) continue;
    if ((players[k].id ?? players[k].playerId) === pid) {
      return { faith: faith / 256, pantheon: b === 0xffffffff ? null : pantheonHashes.get(b) };
    }
    k += 1;
    i += 27;
  }
  return null;
}
// Resources the parser's dictionary lacks (BBG/BBM additions): resolve the raw hash with the ruleset's names.
const RESOURCES = new Map((process.env.RESOURCES || '').split(',').filter(Boolean).map((n) => [civ6Hash(n) >>> 0, n]));
// Game (world) era: per player the save holds a pair (player's era, game era) of ERA_* hashes; the first such
// pair's second hash is the game era (flips at the same turns as the game's tech-cost adjustments).
const ERA_NAMES = ['ERA_ANCIENT', 'ERA_CLASSICAL', 'ERA_MEDIEVAL', 'ERA_RENAISSANCE', 'ERA_INDUSTRIAL', 'ERA_MODERN',
  'ERA_ATOMIC', 'ERA_INFORMATION', 'ERA_FUTURE'];
const ERAS = new Map(ERA_NAMES.map((n) => [civ6Hash(n) >>> 0, n]));
function gameEra(buf) {
  for (let o = 0; o + 8 <= buf.length; o++) {
    const a = ERAS.get(buf.readUInt32LE(o));
    if (a) {
      const b = ERAS.get(buf.readUInt32LE(o + 4));
      if (b) return b;
    }
  }
  return null;
}
const PANTHEONS = new Map((process.env.PANTHEONS || '').split(',').filter(Boolean).map((n) => [civ6Hash(n) >>> 0, n]));
const RELIGIONS = new Map((process.env.RELIGIONS || '').split(',').filter(Boolean).map((n) => [civ6Hash(n) >>> 0, n]));
const BELIEFS = new Map((process.env.BELIEFS || '').split(',').filter(Boolean).map((n) => [civ6Hash(n) >>> 0, n]));

// Founded religions: `religion, founder, ?, ?, ?, ?, nBeliefs, belief...` (found next to the belief list).
function religions(buf) {
  const out = new Map();
  for (let o = 0; o + 48 <= buf.length; o++) {
    const rel = RELIGIONS.get(buf.readUInt32LE(o));
    if (!rel || rel === 'RELIGION_PANTHEON' || out.has(rel)) continue;
    const n = buf.readInt32LE(o + 24);
    if (n < 1 || n > 6) continue;
    const beliefs = [];
    for (let k = 0; k < n; k++) {
      const b = BELIEFS.get(buf.readUInt32LE(o + 28 + 4 * k));
      if (!b) break;
      beliefs.push(b);
    }
    if (beliefs.length !== n) continue;
    out.set(rel, { religion: rel, founder: buf.readInt32LE(o + 4), beliefs });
  }
  return [...out.values()];
}

// A city's religion block, ~6 KB after its LOC_CITY_NAME string: `5, majority (or -1), n` then n entries of
// 17 bytes `5, religion (-1 = none), followers, pressure, flag`. Followers add up to the population.
function cityReligion(buf, at, population) {
  if (at < 0) return null;
  const relOrNone = (v) => (v === 0xffffffff ? null : RELIGIONS.get(v));
  for (let o = at + 5000; o < at + 7500 && o + 12 <= buf.length; o++) {
    if (buf.readInt32LE(o) !== 5) continue;
    const head = buf.readUInt32LE(o + 4);
    if (head !== 0xffffffff && !RELIGIONS.has(head)) continue;
    const n = buf.readInt32LE(o + 8);
    if (n < 1 || n > 16 || o + 12 + 17 * n > buf.length) continue;
    const followers = {};
    let total = 0, ok = true;
    for (let k = 0; k < n; k++) {
      const e = o + 12 + 17 * k;
      const r = buf.readUInt32LE(e + 4);
      if (buf.readInt32LE(e) !== 5 || (r !== 0xffffffff && !RELIGIONS.has(r))) { ok = false; break; }
      const f = buf.readInt32LE(e + 8);
      total += f;
      followers[relOrNone(r) ?? 'NONE'] = f;
    }
    if (!ok || total !== population) continue;
    return { majority: relOrNone(head) ?? null, followers };
  }
  return null;
}

const fs = require('fs'), path = require('path');
const dir = process.argv[2], pid = Number(process.argv[3] ?? 0);

// Water bodies of at most LAKE_MAX_AREA_SIZE (9) plots are lakes; the save has no lake flag, so flood-fill.
const lakeCache = new WeakMap();
function isLake(g, x, y) {
  if (!lakeCache.has(g)) {
    const W = g.map.width, H = g.map.height, water = (p) => p.terrain === 'TERRAIN_COAST' || p.terrain === 'TERRAIN_OCEAN';
    const lake = new Set(), seen = new Set();
    for (const p of g.map.plots) {
      if (!water(p) || seen.has(p.index)) continue;
      const body = [p.index]; seen.add(p.index);
      for (let i = 0; i < body.length; i++) {
        const q = g.map.plots[body[i]];
        for (const [nx, ny] of neighbours(g, q.x, q.y)) {
          const n = ny * W + nx;
          if (!seen.has(n) && water(g.map.plots[n])) { seen.add(n); body.push(n); }
        }
      }
      if (body.length <= 9) body.forEach((n) => lake.add(n));
    }
    lakeCache.set(g, lake);
  }
  return lakeCache.get(g).has(y * g.map.width + x);
}

const plotInfo = (g, x, y) => {
  const p = g.map.plots[y * g.map.width + x];
  const resource = p.resource ?? (p.resourceHash != null ? RESOURCES.get(p.resourceHash >>> 0) ?? null : null);
  return {
    x, y, terrain: p.terrain, feature: p.feature, resource, improvement: p.improvement,
    river: p.riverMap !== 0, pillaged: p.pillaged, district: p.owner?.districtInstanceId ?? null,
    wonder: p.owner?.worldWonder ?? null, // world wonder placed here (finished or not)
    lake: isLake(g, x, y),
    yieldChanges: p.yieldChanges ?? null, // extra yields a flood or eruption left on the tile
  };
};

// Hex neighbours in Civ6's odd-r offset layout (odd rows shifted right); x wraps around.
function neighbours(g, x, y) {
  const d = (y & 1) ? [[1, 0], [-1, 0], [0, -1], [1, -1], [0, 1], [1, 1]] : [[1, 0], [-1, 0], [-1, -1], [0, -1], [-1, 1], [0, 1]];
  return d.map(([dx, dy]) => [((x + dx) % g.map.width + g.map.width) % g.map.width, y + dy])
    .filter(([, ny]) => ny >= 0 && ny < g.map.height);
}

// Active trade routes: the Trader's MAKE_TRADE_ROUTE operation record, found by its shape
// (hash, 2, -1, 0, turn, id, -1, 1, 0, destX, destY, originX, originY, -1); the parser's own lookup
// misses them on these saves.
function tradeRoutes(g, payload, districtAt) {
  const h = civ6Hash('UNITOPERATION_MAKE_TRADE_ROUTE') >>> 0;
  const centers = new Map(g.cities.map((c) => [`${c.center.x},${c.center.y}`, c]));
  const players = Array.isArray(g.players) ? g.players : Object.values(g.players);
  const cityInfo = (c) => routeCityInfo(g, c, districtAt, players);
  const _unused = (c) => {
    const pl = players.find((q) => q.playerId === c.playerId);
    const districts = g.map.plots.filter((q) => q.owner?.playerId === c.playerId && q.owner?.citySlot === c.slot)
      .map((q) => districtAt.get(`${q.x},${q.y}`)).filter((d) => d && d.complete).map((d) => d.type);
    const plots = g.map.plots.filter((q) => q.owner?.playerId === c.playerId && q.owner?.citySlot === c.slot);
    return { name: c.name, id: c.id, playerId: c.playerId, civilization: pl?.civilization ?? null, leader: pl?.leader ?? null,
      x: c.center.x, y: c.center.y, districts,
      // techs and civics known by the city's owner (Grand Embassy compares progress)
      techCount: pl?.techsResearched?.length ?? 0, civicCount: pl?.civicsCompleted?.length ?? 0,
      // what route bonuses count in a city: its improvements, resources and terrain
      plots: plots.map((q) => { const i = plotInfo(g, q.x, q.y); return [i.terrain, i.resource, i.improvement]; }) };
  };
  const out = [];
  for (let o = 0; o + 56 <= payload.length; o++) {
    if (payload.readUInt32LE(o) !== h) continue;
    const r = Array.from({ length: 13 }, (_, j) => payload.readInt32LE(o + 4 + j * 4));
    if (r[0] !== 2 || r[1] !== -1 || r[5] !== -1) continue;
    const origin = centers.get(`${r[10]},${r[11]}`), dest = centers.get(`${r[8]},${r[9]}`);
    if (!origin || !dest) continue;
    out.push({ started: r[3], origin: cityInfo(origin), destination: cityInfo(dest) });
  }
  return out;
}

function routeCityInfo(g, c, districtAt, players) {
  const pl = players.find((q) => q.playerId === c.playerId);
  const own = g.map.plots.filter((q) => q.owner?.playerId === c.playerId && q.owner?.citySlot === c.slot);
  const districts = own.map((q) => districtAt.get(`${q.x},${q.y}`)).filter((d) => d && d.complete).map((d) => d.type);
  return { name: c.name, id: c.id, playerId: c.playerId, civilization: pl?.civilization ?? null, leader: pl?.leader ?? null,
    x: c.center.x, y: c.center.y, districts,
    techCount: pl?.techsResearched?.length ?? 0, civicCount: pl?.civicsCompleted?.length ?? 0,
    plots: own.map((q) => { const i = plotInfo(g, q.x, q.y); return [i.terrain, i.resource, i.improvement]; }) };
}

function nearPlots(g, c, districtAt) {
  const own = new Set(g.map.plots.filter((q) => q.owner?.playerId === c.playerId && q.owner?.citySlot === c.slot).map((q) => `${q.x},${q.y}`));
  const out = new Map();
  for (const k of own) {
    const [x, y] = k.split(',').map(Number);
    for (const [nx, ny] of neighbours(g, x, y)) {
      const key = `${nx},${ny}`;
      if (own.has(key) || out.has(key)) continue;
      const q = g.map.plots[ny * g.map.width + nx];
      out.set(key, { ...plotInfo(g, nx, ny), districtType: districtAt.get(key)?.type ?? null, owner: q.owner?.playerId ?? null });
    }
  }
  return [...out.values()];
}

for (const f of fs.readdirSync(dir).filter((f) => f.endsWith('.Civ6Save')).sort()) {
  const raw = fs.readFileSync(path.join(dir, f));
  let g;
  try {
    g = parseCiv6Save(raw);
  } catch (err) {
    console.error(`skipping ${f}: ${err.message}`);  // e.g. a save written while the game was mid-turn
    continue;
  }
  const payload = decompressCiv6Payload(raw);
  const cityYieldRuns = cityYieldBlocks(payload);
  const allPlayers = (Array.isArray(g.players) ? g.players : Object.values(g.players)).slice().sort((x, y) => (x.id ?? x.playerId) - (y.id ?? y.playerId));
  const religion = playerReligion(payload, PANTHEONS, allPlayers, pid);
  // this player's run of city yields: the players with cities in id order
  const withCities = [...new Set(g.cities.map((c) => c.playerId))].sort((a, b) => a - b);
  const myRun = cityYieldRuns[withCities.indexOf(pid)];
  const myCityCount = g.cities.filter((c) => c.playerId === pid).length;
  // Every district on the map, from the raw district records matched to tiles by coordinates (the parser's
  // per-city lists drop districts when city ids repeat across players). Records carry the district's cost.
  const plotDistrictId = new Map(g.map.plots.filter((q) => q.owner?.districtInstanceId != null)
    .map((q) => [`${q.x},${q.y}`, q.owner.districtInstanceId >>> 0]));
  const districtAt = districtRecords(payload, plotDistrictId);
  const cityDistricts = (c) => g.map.plots
    .filter((q) => q.owner?.playerId === c.playerId && q.owner?.citySlot === c.slot && districtAt.has(`${q.x},${q.y}`))
    .map((q) => ({ ...districtAt.get(`${q.x},${q.y}`), x: q.x, y: q.y }));
  // City food store (x256): the int after the marker `0, 0, 23`, about 5.8 KB past the first
  // `LOC_CITY_NAME_<NAME>` string (found on BBG saves; the parser's own food field reads a constant).
  const foodStore = (c) => {
    const at = payload.indexOf(Buffer.from(`LOC_CITY_NAME_${c.name}`));
    if (at < 0) return null;
    for (let o = at + 5000; o < at + 7000 && o + 8 <= payload.length; o++) {
      if (payload.readInt32LE(o) === 23 && payload.readInt32LE(o - 4) === 0 && payload.readInt32LE(o - 8) === 0) {
        return payload.readInt32LE(o + 4) / 256;
      }
    }
    return null;
  };
  const p = (Array.isArray(g.players) ? g.players : Object.values(g.players)).find((p) => p.playerId === pid);
  const cities = g.cities.filter((c) => c.playerId === pid).map((c) => ({
    name: c.name, id: c.id, capital: c.capital, slot: c.slot, center: c.center, population: c.population, foodStore: foodStore(c),
    religion: cityReligion(payload, payload.indexOf(Buffer.from(`LOC_CITY_NAME_${c.name}`)), c.population),
    worked: c.workedPlots.map((w) => ({ ...plotInfo(g, w.x, w.y), workers: w.workers })),
    owned: g.map.plots.filter((q) => q.owner?.playerId === pid && q.owner?.citySlot === c.slot).map((q) => plotInfo(g, q.x, q.y)),
    // Plots next to the city's tiles that it doesn't own (needed for appeal and adjacency), with district types.
    near: nearPlots(g, c, districtAt),
    production: { current: c.production.current, progress: c.production.progress },
    buildings: c.buildings.map((b) => b.type),
    // great works on display: [building, work type]
    greatWorks: (c.greatWorkSlots ?? []).filter((x) => x.workIndex != null && g.greatWorks?.[x.workIndex])
      .map((x) => [x.building, g.greatWorks[x.workIndex].type]),
    districts: cityDistricts(c).map((d) => ({
      type: d.type, x: d.x, y: d.y, complete: d.complete, cost: d.cost, plot: plotInfo(g, d.x, d.y),
      neighbours: neighbours(g, d.x, d.y).map(([nx, ny]) => ({ ...plotInfo(g, nx, ny), districtType: districtAt.get(`${nx},${ny}`)?.type ?? null })),
    })),
  }));
  const out = {
    file: f, mapWidth: g.map.width, turn: g.metadata.turn, gameSpeed: g.metadata.gameSpeed, gameEra: gameEra(payload),
    player: {
      playerId: pid, civilization: p.civilization, leader: p.leader, eraScore: p.eraScore, dedications: p.dedications?.active ?? [],
      // major civilizations met: one MET_MAJOR moment per meeting (the last one is recorded as MET_ALL_MAJORS)
      majorsMet: (g.moments ?? []).filter((m) => m.playerId === pid && /^MOMENT_PLAYER_MET_(MAJOR|ALL_MAJORS)/.test(m.type)).length,
      // the current age, from the latest MOMENT_GAME_ERA_STARTED_WITH_<X>_AGE moment (NORMAL/GOLDEN/DARK/HEROIC)
      age: ((g.moments ?? []).filter((m) => m.playerId === pid && /^MOMENT_GAME_ERA_STARTED_WITH_/.test(m.type)).pop()?.type
        .replace('MOMENT_GAME_ERA_STARTED_WITH_', '').replace('_AGE', '')) ?? 'NORMAL',
      gold: p.gold, goldPerTurn: p.goldPerTurn, faith: p.faith,
      yields: p.yields, techs: p.techsResearched, techsBoosted: p.techsBoosted, techProgress: p.techProgress,
      currentResearch: p.currentResearch, civics: p.civicsCompleted, civicsInspired: p.civicsInspired,
      civicProgress: p.civicProgress, currentCivic: p.currentCivic, government: p.government,
      policies: p.policiesSlotted.map((x) => x.policy), unitsTrained: p.unitsTrained, goodyHutsReceived: p.goodyHutsReceived,
      faithStock: religion?.faith ?? p.faith, pantheon: religion?.pantheon ?? p.pantheon,
      // great person points banked / per turn per class; prophet points turn into faith once no prophet is on offer
      greatPersonPoints: p.greatPersonPoints ?? {},
      prophetOnOffer: (g.greatPeople ?? []).some((x) => x.class === 'GREAT_PERSON_CLASS_PROPHET' && x.recruitedBy == null),
      // great people this player recruited (individual, class, turn); the AI uses them at once
      greatPeopleRecruited: (g.greatPeople ?? []).filter((x) => x.recruitedBy === pid)
        .map((x) => ({ individual: x.individual, cls: x.class, turn: x.recruitedTurn })),
      // the first unclaimed individual of each class (who is on offer now)
      greatPeopleOnOffer: (() => { const out = {}; for (const x of g.greatPeople ?? []) if (x.recruitedBy == null && !out[x.class]) out[x.class] = x.individual; return out; })(),
      prophetRecruitedTurn: (g.greatPeople ?? []).find((x) => x.class === 'GREAT_PERSON_CLASS_PROPHET' && x.recruitedBy === pid)?.recruitedTurn ?? null,
      governors: (p.governors ?? []).map((v) => ({ type: v.type, promotions: v.promotions, cityId: v.cityId, cityOwnerId: v.cityOwnerId, established: v.established })),
      envoys: Object.entries(p.envoys ?? {}).map(([id, n]) => {
        const all = Array.isArray(g.players) ? g.players : Object.values(g.players);
        const cs = all.find((q) => q.playerId === Number(id));
        // suzerain: at least 3 envoys and more than any other player (ties keep the old suzerain; counted as not)
        const others = all.filter((q) => q.playerId !== pid).map((q) => (q.envoys ?? {})[id] ?? 0);
        const suzerain = n >= 3 && others.every((m) => m < n);
        // luxuries the city-state has improved (a suzerain receives them)
        const luxuries = g.map.plots.filter((q) => q.owner?.playerId === Number(id) && q.improvement && !q.pillaged)
          .map((q) => plotInfo(g, q.x, q.y).resource).filter(Boolean);
        return { playerId: Number(id), civilization: cs?.civilization, leader: cs?.leader, count: n, suzerain, resources: luxuries };
      }),
    },
    cities,
    cityYields: myRun && myRun.length === myCityCount ? myRun : null,
    units: g.units.filter((u) => u.playerId === pid).map((u) => ({ type: u.type, x: u.x, y: u.y, id: u.id ?? null,
      charges: u.builderCharges ?? null })),
    // founded religions with their beliefs, and world-wide followers / cities following each
    religions: (() => {
      const rs = religions(payload);
      for (const r of rs) { r.followers = 0; r.cities = 0; r.cityList = []; }
      for (const c of g.cities) {
        const cr = cityReligion(payload, payload.indexOf(Buffer.from(`LOC_CITY_NAME_${c.name}`)), c.population);
        if (!cr) continue;
        const own = g.map.plots.filter((q) => q.owner?.playerId === c.playerId && q.owner?.citySlot === c.slot);
        for (const r of rs) {
          const n = cr.followers[r.religion] ?? 0;
          r.followers += n;
          if (cr.majority === r.religion) r.cities += 1;
          // founder beliefs count followers, following cities, their districts and wonders, at home and abroad
          if (n > 0 || cr.majority === r.religion) {
            r.cityList.push({ name: c.name, playerId: c.playerId, followers: n, majority: cr.majority === r.religion,
              districts: own.map((q) => districtAt.get(`${q.x},${q.y}`)).filter((d) => d && d.complete).map((d) => d.type),
              wonders: own.map((q) => plotInfo(g, q.x, q.y).wonder).filter((w) => w && (c.buildings ?? []).some((b) => b.type === w)).length });
          }
        }
      }
      return rs;
    })(),
    // every city in the world (possible trade route destinations), in the route endpoint format
    worldCities: (() => { const players = Array.isArray(g.players) ? g.players : Object.values(g.players);
      return g.cities.filter((c) => c.playerId !== pid).map((c) => routeCityInfo(g, c, districtAt, players)); })(),
    // world wonders finished anywhere (a plan can't build one another civ has finished)
    worldWonders: g.cities.flatMap((c) => (c.buildings ?? []).filter((b) => WONDERS.has(b.type))
      .map((b) => ({ building: b.type, playerId: c.playerId, city: c.name }))),
    // routes starting or ending in this player's cities
    tradeRoutes: tradeRoutes(g, payload, districtAt).filter((t) => t.origin.playerId === pid || t.destination.playerId === pid),
  };
  console.log(JSON.stringify(out));
}
