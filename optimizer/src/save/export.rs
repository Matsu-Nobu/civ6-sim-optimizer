//! The export rows (one JSON object per save) and the map export, from parsed saves. Ported from the project's
//! former save_export/export_game.js and export_map.js; the field notes are theirs.
use super::{civ6_hash, Bytes, City, Dict, Plot, Save};
use crate::json::Value;
use std::collections::{HashMap, HashSet};

/// Names from the rules database the export resolves raw hashes with.
pub struct Names {
    pub resources: HashMap<u32, String>,
    pub religions: HashMap<u32, String>,
    pub beliefs: HashMap<u32, String>,
    pub pantheons: HashMap<u32, String>,
    pub wonders: HashSet<String>,
}

fn by_hash(db: &rusqlite::Connection, sql: &str) -> HashMap<u32, String> {
    let mut q = db.prepare(sql).unwrap();
    q.query_map([], |r| r.get::<_, String>(0)).unwrap().map(|n| n.unwrap()).map(|n| (civ6_hash(&n), n)).collect()
}

impl Names {
    pub fn load(db: &rusqlite::Connection) -> Names {
        Names {
            resources: by_hash(db, "select ResourceType from Resources"),
            religions: by_hash(db, "select ReligionType from Religions"),
            beliefs: by_hash(db, "select BeliefType from Beliefs"),
            pantheons: by_hash(db, "select BeliefType from Beliefs where BeliefClassType='BELIEF_CLASS_PANTHEON'"),
            wonders: by_hash(db, "select BuildingType from Buildings where IsWonder=1").into_values().collect(),
        }
    }
}

/// The rules database's types, (name, kind), for the dictionary.
pub fn rules_types(db: &rusqlite::Connection) -> Vec<(String, String)> {
    let mut q = db.prepare("select Type, Kind from Types").unwrap();
    q.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|x| x.unwrap()).collect()
}

fn s(v: &str) -> Value {
    Value::from(v)
}

fn opt(v: Option<&str>) -> Value {
    v.map(Value::from).unwrap_or(Value::Null)
}

fn xy(x: i64, y: i64) -> Value {
    Value::obj(vec![("x", Value::from(x)), ("y", Value::from(y))])
}

fn changes(c: &Option<Vec<(String, f64)>>) -> Value {
    c.as_ref().map(|c| Value::Obj(c.iter().map(|(k, v)| (k.clone(), Value::from(*v))).collect())).unwrap_or(Value::Null)
}

fn named(v: &[(&'static str, f64)]) -> Value {
    Value::Obj(v.iter().map(|(k, x)| (k.to_string(), Value::from(*x))).collect())
}

fn list(v: &[&'static str]) -> Value {
    Value::Arr(v.iter().map(|x| Value::from(*x)).collect())
}

/// The map export: every plot's terrain, feature, resource, improvement, river and owner.
pub fn map_json(g: &Save, n: &Names) -> Value {
    let plots = g.map.plots.iter().map(|p| Value::obj(vec![
        ("x", Value::from(p.x)), ("y", Value::from(p.y)), ("terrain", s(&p.terrain)), ("feature", opt(p.feature.as_deref())),
        ("resource", opt(resource(p, n).as_deref())), ("improvement", opt(p.improvement.as_deref())),
        ("river", Value::from(p.river_map != 0)), ("riverEdges", Value::from(p.river_map as i64)),
        ("route", p.route_index.map(Value::from).unwrap_or(Value::Null)),
        ("owner", p.owner.as_ref().map(|o| Value::from(o.player_id)).unwrap_or(Value::Null)), ("yieldChanges", changes(&p.yield_changes)),
    ])).collect();
    Value::obj(vec![("turn", Value::from(g.turn)), ("width", Value::from(g.map.width)), ("height", Value::from(g.map.height)), ("plots", Value::Arr(plots))])
}

/// A plot's resource: the parser's name, else the rules database's name for its raw hash (mod-added resources).
fn resource(p: &Plot, n: &Names) -> Option<String> {
    p.resource.clone().or_else(|| n.resources.get(&p.resource_hash).cloned())
}

struct Ctx<'a> {
    g: &'a Save,
    n: &'a Names,
    lake: HashSet<i64>,
    district_at: HashMap<(i64, i64), (&'static str, bool, i64)>,
}

impl<'a> Ctx<'a> {
    fn plot(&self, x: i64, y: i64) -> &'a Plot {
        &self.g.map.plots[(y * self.g.map.width + x) as usize]
    }
    /// Hex neighbours in Civ6's odd-r offset layout (odd rows shifted right); x wraps around.
    fn neighbours(&self, x: i64, y: i64) -> Vec<(i64, i64)> {
        let w = self.g.map.width;
        let d: [(i64, i64); 6] = if y & 1 == 1 { [(1, 0), (-1, 0), (0, -1), (1, -1), (0, 1), (1, 1)] } else { [(1, 0), (-1, 0), (-1, -1), (0, -1), (-1, 1), (0, 1)] };
        d.iter().map(|(dx, dy)| (((x + dx) % w + w) % w, y + dy)).filter(|(_, ny)| *ny >= 0 && *ny < self.g.map.height).collect()
    }
    fn plot_info(&self, x: i64, y: i64) -> Vec<(String, Value)> {
        let p = self.plot(x, y);
        vec![
            ("x".into(), Value::from(x)), ("y".into(), Value::from(y)), ("terrain".into(), s(&p.terrain)), ("feature".into(), opt(p.feature.as_deref())),
            ("resource".into(), opt(resource(p, self.n).as_deref())), ("improvement".into(), opt(p.improvement.as_deref())),
            ("river".into(), Value::from(p.river_map != 0)), ("pillaged".into(), Value::from(p.pillaged)),
            ("district".into(), p.owner.as_ref().and_then(|o| o.district_instance_id).map(|d| Value::from(d as i64)).unwrap_or(Value::Null)),
            // world wonder placed here (finished or not)
            ("wonder".into(), opt(p.owner.as_ref().and_then(|o| o.world_wonder.as_deref()))),
            ("lake".into(), Value::from(self.lake.contains(&(y * self.g.map.width + x)))),
            // extra yields a flood or eruption left on the tile
            ("yieldChanges".into(), changes(&p.yield_changes)),
        ]
    }
    fn plot_value(&self, x: i64, y: i64, extra: Vec<(&str, Value)>) -> Value {
        let mut o = self.plot_info(x, y);
        o.extend(extra.into_iter().map(|(k, v)| (k.to_string(), v)));
        Value::Obj(o)
    }
    fn own(&self, c: &City) -> Vec<&'a Plot> {
        self.g.map.plots.iter().filter(|q| q.owner.as_ref().map(|o| o.player_id == c.player_id && o.city_slot == c.slot).unwrap_or(false)).collect()
    }
    fn district_type(&self, x: i64, y: i64) -> Value {
        opt(self.district_at.get(&(x, y)).map(|d| d.0))
    }
    fn player(&self, id: i64) -> Option<&'a super::Player> {
        self.g.players.iter().find(|p| p.state.player_id == id)
    }
    fn route_city(&self, c: &City) -> Value {
        let pl = self.player(c.player_id);
        let own = self.own(c);
        let districts: Vec<Value> = own.iter().filter_map(|q| self.district_at.get(&(q.x, q.y))).filter(|d| d.1).map(|d| s(d.0)).collect();
        Value::obj(vec![
            ("name", s(&c.name)), ("id", Value::from(c.id)), ("playerId", Value::from(c.player_id)),
            ("civilization", pl.map(|p| s(&p.civilization)).unwrap_or(Value::Null)), ("leader", pl.map(|p| s(&p.leader)).unwrap_or(Value::Null)),
            ("x", Value::from(c.center.0)), ("y", Value::from(c.center.1)), ("districts", Value::Arr(districts)),
            // techs and civics known by the city's owner (Grand Embassy compares progress)
            ("techCount", Value::from(pl.map(|p| p.state.techs_researched.len()).unwrap_or(0))),
            ("civicCount", Value::from(pl.map(|p| p.state.civics_completed.len()).unwrap_or(0))),
            // what route bonuses count in a city: its improvements, resources and terrain
            ("plots", Value::Arr(own.iter().map(|q| Value::Arr(vec![s(&q.terrain), opt(resource(q, self.n).as_deref()), opt(q.improvement.as_deref())])).collect())),
        ])
    }
}

/// Water bodies of at most 9 plots (LAKE_MAX_AREA_SIZE) are lakes; the save has no lake flag, so flood-fill.
fn lakes(g: &Save) -> HashSet<i64> {
    let (w, h) = (g.map.width, g.map.height);
    let water = |p: &Plot| p.terrain == "TERRAIN_COAST" || p.terrain == "TERRAIN_OCEAN";
    let nb = |x: i64, y: i64| -> Vec<(i64, i64)> {
        let d: [(i64, i64); 6] = if y & 1 == 1 { [(1, 0), (-1, 0), (0, -1), (1, -1), (0, 1), (1, 1)] } else { [(1, 0), (-1, 0), (-1, -1), (0, -1), (-1, 1), (0, 1)] };
        d.iter().map(|(dx, dy)| (((x + dx) % w + w) % w, y + dy)).filter(|(_, ny)| *ny >= 0 && *ny < h).collect()
    };
    let mut lake = HashSet::new();
    let mut seen = HashSet::new();
    for p in &g.map.plots {
        if !water(p) || seen.contains(&p.index) {
            continue;
        }
        let mut body = vec![p.index];
        seen.insert(p.index);
        let mut i = 0;
        while i < body.len() {
            let q = &g.map.plots[body[i] as usize];
            for (nx, ny) in nb(q.x, q.y) {
                let k = ny * w + nx;
                if !seen.contains(&k) && water(&g.map.plots[k as usize]) {
                    seen.insert(k);
                    body.push(k);
                }
            }
            i += 1;
        }
        if body.len() <= 9 {
            lake.extend(body);
        }
    }
    lake
}

/// District records `15, instanceId, x, y, cityId, DISTRICT_* hash, damage, wallsDamage, lastDamagedTurn, 0, cost,
/// built`, accepted when the id and the coordinates match a tile's district id.
fn district_records(p: &Bytes, d: &Dict, plot_id: &HashMap<(i64, i64), u32>) -> HashMap<(i64, i64), (&'static str, bool, i64)> {
    let mut out = HashMap::new();
    let mut o = 0;
    while o + 48 <= p.len() {
        if p.i32(o) == 15 {
            let key = (p.i32(o + 8) as i64, p.i32(o + 12) as i64);
            let id = p.u32(o + 4);
            if plot_id.get(&key) == Some(&id) && !out.contains_key(&key) {
                if let Some(t) = d.resolve(p.u32(o + 20)).filter(|t| t.kind == "KIND_DISTRICT") {
                    out.insert(key, (t.name, p.i32(o + 44) == 1, p.i32(o + 40) as i64));
                }
            }
        }
        o += 1;
    }
    out
}

/// Per-city yields: `cityId, 0x11, ...` then two maps `6, (yieldHash, value×256) × 6`, after and before percentage
/// modifiers; the runs of ids 0, 1, 2, … belong to the players with cities, in player id order.
fn city_yield_runs(p: &Bytes) -> Vec<Vec<Value>> {
    let names = ["YIELD_FOOD", "YIELD_PRODUCTION", "YIELD_GOLD", "YIELD_SCIENCE", "YIELD_CULTURE", "YIELD_FAITH"];
    let by: HashMap<u32, &str> = names.iter().map(|n| (civ6_hash(n), *n)).collect();
    let read_map = |i: i64| -> Option<Value> {
        if p.u32(i) != 6 {
            return None;
        }
        let mut m = vec![];
        for k in 0..6 {
            let y = by.get(&p.u32(i + 4 + k * 8))?;
            m.push((y.to_string(), Value::from(p.i32(i + 8 + k * 8) as f64 / 256.0)));
        }
        Some(Value::Obj(m))
    };
    let mut runs: Vec<Vec<Value>> = vec![];
    let mut i = 28;
    while i + 110 < p.len() {
        if p.u32(i) == 6 {
            let tag = p.u32(i - 24);
            if tag == 0x10 || tag == 0x11 {
                if let Some(a) = read_map(i) {
                    if let Some(b) = read_map(i + 52) {
                        let id = p.u32(i - 28) as usize;
                        if id == 0 {
                            runs.push(vec![]);
                        }
                        if let Some(last) = runs.last_mut().filter(|l| l.len() == id) {
                            last.push(Value::obj(vec![("modified", a), ("base", b)]));
                        }
                        i += 103;
                    }
                }
            }
        }
        i += 1;
    }
    runs
}

/// Faith and pantheon: `20, faith×256, 0, BELIEF_*|-1, n, x, y`, the records in player id order, matched to the
/// players one by one by faith.
fn player_religion(p: &Bytes, n: &Names, players: &[&super::Player], pid: i64) -> Option<(f64, Option<String>)> {
    let mut k = 0;
    let mut i = 0;
    while i + 28 < p.len() && k < players.len() {
        if p.i32(i) == 20 && p.i32(i + 8) == 0 {
            let b = p.u32(i + 12);
            if b == 0xffffffff || n.pantheons.contains_key(&b) {
                let faith = p.i32(i + 4);
                if (0..=1_000_000 * 256).contains(&faith) && players[k].state.faith.map(|f| (faith as f64 / 256.0 - f).abs() <= 1e-6).unwrap_or(true) {
                    if players[k].state.player_id == pid {
                        return Some((faith as f64 / 256.0, if b == 0xffffffff { None } else { n.pantheons.get(&b).cloned() }));
                    }
                    k += 1;
                    i += 27;
                }
            }
        }
        i += 1;
    }
    None
}

/// Game (world) era: the first pair of ERA_* hashes (player's era, game era); the second is the game era.
fn game_era(p: &Bytes) -> Value {
    let eras = ["ERA_ANCIENT", "ERA_CLASSICAL", "ERA_MEDIEVAL", "ERA_RENAISSANCE", "ERA_INDUSTRIAL", "ERA_MODERN", "ERA_ATOMIC", "ERA_INFORMATION", "ERA_FUTURE"];
    let by: HashMap<u32, &str> = eras.iter().map(|n| (civ6_hash(n), *n)).collect();
    let mut o = 0;
    while o + 8 <= p.len() {
        if by.contains_key(&p.u32(o)) {
            if let Some(b) = by.get(&p.u32(o + 4)) {
                return s(b);
            }
        }
        o += 1;
    }
    Value::Null
}

/// Founded religions: `religion, founder, ?, ?, ?, ?, nBeliefs, belief...`.
fn religions(p: &Bytes, n: &Names) -> Vec<(String, i64, Vec<String>)> {
    let mut out: Vec<(String, i64, Vec<String>)> = vec![];
    let mut o = 0;
    while o + 48 <= p.len() {
        if let Some(rel) = n.religions.get(&p.u32(o)).filter(|r| *r != "RELIGION_PANTHEON" && !out.iter().any(|x| &x.0 == *r)) {
            let k = p.i32(o + 24);
            if (1..=6).contains(&k) {
                let beliefs: Vec<String> = (0..k as i64).map_while(|j| n.beliefs.get(&p.u32(o + 28 + 4 * j)).cloned()).collect();
                if beliefs.len() == k as usize {
                    out.push((rel.clone(), p.i32(o + 4) as i64, beliefs));
                }
            }
        }
        o += 1;
    }
    out
}

/// A city's religion block, ~6 KB after its LOC_CITY_NAME string: `5, majority (or -1), n` then n entries of 17
/// bytes `5, religion (-1 = none), followers, pressure, flag`; the followers add up to the population.
fn city_religion(p: &Bytes, n: &Names, at: i64, population: i64) -> Option<(Option<String>, Vec<(String, i64)>)> {
    if at < 0 {
        return None;
    }
    let rel = |v: u32| if v == 0xffffffff { None } else { n.religions.get(&v).cloned() };
    let mut o = at + 5000;
    while o < at + 7500 && o + 12 <= p.len() {
        let here = o;
        o += 1;
        if p.i32(here) != 5 {
            continue;
        }
        let head = p.u32(here + 4);
        if head != 0xffffffff && !n.religions.contains_key(&head) {
            continue;
        }
        let k = p.i32(here + 8) as i64;
        if !(1..=16).contains(&k) || here + 12 + 17 * k > p.len() {
            continue;
        }
        let mut followers: Vec<(String, i64)> = vec![];
        let mut total = 0;
        let mut ok = true;
        for j in 0..k {
            let e = here + 12 + 17 * j;
            let r = p.u32(e + 4);
            if p.i32(e) != 5 || (r != 0xffffffff && !n.religions.contains_key(&r)) {
                ok = false;
                break;
            }
            let f = p.i32(e + 8) as i64;
            total += f;
            let key = rel(r).unwrap_or("NONE".into());
            match followers.iter_mut().find(|x| x.0 == key) {
                Some(x) => x.1 = f,
                None => followers.push((key, f)),
            }
        }
        if !ok || total != population {
            continue;
        }
        return Some((rel(head), followers));
    }
    None
}

/// City food store ×256: the int after the marker `0, 0, 23`, about 5.8 KB past the first LOC_CITY_NAME_<NAME>.
fn food_store(p: &Bytes, name: &str) -> Value {
    let at = p.index_of(format!("LOC_CITY_NAME_{name}").as_bytes(), 0);
    if at < 0 {
        return Value::Null;
    }
    let mut o = at + 5000;
    while o < at + 7000 && o + 8 <= p.len() {
        if p.i32(o) == 23 && p.i32(o - 4) == 0 && p.i32(o - 8) == 0 {
            return Value::from(p.i32(o + 4) as f64 / 256.0);
        }
        o += 1;
    }
    Value::Null
}

/// Active trade routes: the Trader's MAKE_TRADE_ROUTE operation record `hash, 2, -1, 0, turn, id, -1, 1, 0, destX,
/// destY, originX, originY, -1`.
fn trade_routes(c: &Ctx, p: &Bytes, pid: i64) -> Vec<Value> {
    let h = civ6_hash("UNITOPERATION_MAKE_TRADE_ROUTE");
    let mut centers: HashMap<(i64, i64), &City> = HashMap::new();
    for city in &c.g.cities {
        centers.insert(city.center, city);
    }
    let mut out = vec![];
    let mut o = 0;
    while o + 56 <= p.len() {
        if p.u32(o) == h {
            let r: Vec<i64> = (0..13).map(|j| p.i32(o + 4 + j * 4) as i64).collect();
            if r[0] == 2 && r[1] == -1 && r[5] == -1 {
                if let (Some(origin), Some(dest)) = (centers.get(&(r[10], r[11])), centers.get(&(r[8], r[9]))) {
                    if origin.player_id == pid || dest.player_id == pid {
                        out.push(Value::obj(vec![("started", Value::from(r[3])), ("origin", c.route_city(origin)), ("destination", c.route_city(dest))]));
                    }
                }
            }
        }
        o += 1;
    }
    out
}

fn prefix_matches(s: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|p| s.starts_with(p))
}

/// One export row: the player's state, its cities with their plots, units, religions, the world's cities and
/// wonders, and the trade routes, from one save.
pub fn row_json(g: &Save, d: &Dict, n: &Names, file: &str, pid: i64) -> Value {
    let p = Bytes(&g.payload);
    let plot_id: HashMap<(i64, i64), u32> = g.map.plots.iter().filter_map(|q| q.owner.as_ref().and_then(|o| o.district_instance_id).map(|id| ((q.x, q.y), id))).collect();
    let ctx = Ctx { g, n, lake: lakes(g), district_at: district_records(&p, d, &plot_id) };
    let runs = city_yield_runs(&p);
    let mut all: Vec<&super::Player> = g.players.iter().collect();
    all.sort_by_key(|x| x.state.player_id);
    let religion = player_religion(&p, n, &all, pid);
    let mut with_cities: Vec<i64> = g.cities.iter().map(|c| c.player_id).collect::<HashSet<_>>().into_iter().collect();
    with_cities.sort();
    let my_run = with_cities.iter().position(|x| *x == pid).and_then(|i| runs.get(i));
    let my_count = g.cities.iter().filter(|c| c.player_id == pid).count();
    let me = ctx.player(pid).unwrap_or_else(|| panic!("{file}: no player {pid}"));
    let st = &me.state;
    let name_at = |c: &City| p.index_of(format!("LOC_CITY_NAME_{}", c.name).as_bytes(), 0);
    let cities: Vec<Value> = g.cities.iter().filter(|c| c.player_id == pid).map(|c| {
        let own = ctx.own(c);
        let own_set: HashSet<(i64, i64)> = own.iter().map(|q| (q.x, q.y)).collect();
        // plots next to the city's tiles that it doesn't own (for appeal and adjacency), with district types
        let mut near: Vec<Value> = vec![];
        let mut seen: HashSet<(i64, i64)> = HashSet::new();
        for q in &own {
            for (nx, ny) in ctx.neighbours(q.x, q.y) {
                if own_set.contains(&(nx, ny)) || !seen.insert((nx, ny)) {
                    continue;
                }
                let o = ctx.plot(nx, ny).owner.as_ref().map(|o| Value::from(o.player_id)).unwrap_or(Value::Null);
                near.push(ctx.plot_value(nx, ny, vec![("districtType", ctx.district_type(nx, ny)), ("owner", o)]));
            }
        }
        let rel = city_religion(&p, n, name_at(c), c.population).map(|(m, f)| Value::obj(vec![
            ("majority", m.map(Value::from).unwrap_or(Value::Null)),
            ("followers", Value::Obj(f.into_iter().map(|(k, v)| (k, Value::from(v))).collect())),
        ])).unwrap_or(Value::Null);
        let districts: Vec<Value> = own.iter().filter_map(|q| ctx.district_at.get(&(q.x, q.y)).map(|dd| (q, dd))).map(|(q, dd)| Value::obj(vec![
            ("type", s(dd.0)), ("x", Value::from(q.x)), ("y", Value::from(q.y)), ("complete", Value::from(dd.1)), ("cost", Value::from(dd.2)),
            ("plot", Value::Obj(ctx.plot_info(q.x, q.y))),
            ("neighbours", Value::Arr(ctx.neighbours(q.x, q.y).into_iter().map(|(nx, ny)| ctx.plot_value(nx, ny, vec![("districtType", ctx.district_type(nx, ny))])).collect())),
        ])).collect();
        // great works on display: [building, work type]
        let works: Vec<Value> = c.great_work_slots.iter().filter_map(|(b, w)| w.and_then(|w| g.great_works.get(w as usize)).map(|t| Value::Arr(vec![s(b), s(t)]))).collect();
        Value::obj(vec![
            ("name", s(&c.name)), ("id", Value::from(c.id)), ("capital", Value::from(c.capital)), ("slot", Value::from(c.slot)),
            ("center", xy(c.center.0, c.center.1)), ("population", Value::from(c.population)), ("foodStore", food_store(&p, &c.name)),
            ("religion", rel),
            ("worked", Value::Arr(c.worked.iter().map(|(x, y, w)| ctx.plot_value(*x, *y, vec![("workers", Value::from(*w))])).collect())),
            ("owned", Value::Arr(own.iter().map(|q| Value::Obj(ctx.plot_info(q.x, q.y))).collect())),
            ("near", Value::Arr(near)),
            ("production", Value::obj(vec![("current", opt(c.current)), ("progress", named(&c.progress)),
                ("heldOverflow", c.held_overflow.map(Value::from).unwrap_or(Value::Null))])),
            ("buildings", list(&c.buildings)),
            ("greatWorks", Value::Arr(works)),
            ("districts", Value::Arr(districts)),
        ])
    }).collect();
    let moments_of = |re: &[&str]| g.moments.iter().filter(|m| m.player_id == pid && prefix_matches(m.kind, re)).collect::<Vec<_>>();
    let age = moments_of(&["MOMENT_GAME_ERA_STARTED_WITH_"]).last()
        .map(|m| m.kind.replacen("MOMENT_GAME_ERA_STARTED_WITH_", "", 1).replacen("_AGE", "", 1)).unwrap_or("NORMAL".into());
    let gp = &g.great_people;
    let mut on_offer: Vec<(String, Value)> = vec![];
    for x in gp.iter().filter(|x| x.recruited_by.is_none()) {
        if !on_offer.iter().any(|e| e.0 == x.class) {
            on_offer.push((x.class.to_string(), s(x.individual)));
        }
    }
    let envoys: Vec<Value> = {
        let mut e = me.envoys.clone();
        e.sort();
        e.into_iter().map(|(id, count)| {
            let cs = ctx.player(id);
            // suzerain: at least 3 envoys and more than any other player (ties keep the old suzerain; counted as not)
            let others = g.players.iter().filter(|q| q.state.player_id != pid).map(|q| q.envoys.iter().find(|x| x.0 == id).map(|x| x.1).unwrap_or(0));
            let suzerain = count >= 3 && others.into_iter().all(|m| m < count);
            // luxuries the city-state has improved (a suzerain receives them)
            let lux: Vec<Value> = g.map.plots.iter().filter(|q| q.owner.as_ref().map(|o| o.player_id == id).unwrap_or(false) && q.improvement.is_some() && !q.pillaged)
                .filter_map(|q| resource(q, n)).map(Value::from).collect();
            let mut o = vec![("playerId", Value::from(id))];
            if let Some(cs) = cs {
                o.push(("civilization", s(&cs.civilization)));
                o.push(("leader", s(&cs.leader)));
            }
            o.extend([("count", Value::from(count)), ("suzerain", Value::from(suzerain)), ("atWar", Value::from(me.at_war.contains(&id))), ("resources", Value::Arr(lux))]);
            Value::obj(o)
        }).collect()
    };
    let governors: Vec<Value> = me.governors.iter().map(|v| {
        let mut o = vec![("type", s(v.governor)), ("promotions", list(&v.promotions)),
            ("cityId", v.city_id.map(Value::from).unwrap_or(Value::Null)), ("cityOwnerId", v.city_owner_id.map(Value::from).unwrap_or(Value::Null))];
        if let Some(e) = v.established {
            o.push(("established", Value::from(e)));
        }
        Value::obj(o)
    }).collect();
    let player = Value::obj(vec![
        ("playerId", Value::from(pid)), ("civilization", s(&me.civilization)), ("leader", s(&me.leader)),
        ("eraScore", st.era_score.map(Value::from).unwrap_or(Value::Null)),
        ("score", st.score.map(Value::from).unwrap_or(Value::Null)),
        ("dedications", me.dedications.as_ref().map(|x| list(x)).unwrap_or(Value::Arr(vec![]))),
        // major civilizations met: one MET_MAJOR moment per meeting (the last one is recorded as MET_ALL_MAJORS)
        ("majorsMet", Value::from(moments_of(&["MOMENT_PLAYER_MET_MAJOR", "MOMENT_PLAYER_MET_ALL_MAJORS"]).len())),
        // the current age, from the latest MOMENT_GAME_ERA_STARTED_WITH_<X>_AGE moment (NORMAL/GOLDEN/DARK/HEROIC)
        ("age", Value::from(age)),
        ("gold", Value::from(st.gold)), ("goldPerTurn", Value::from(st.gold_per_turn)), ("faith", st.faith.map(Value::from).unwrap_or(Value::Null)),
        ("yields", named(&st.yields)), ("techs", list(&st.techs_researched)), ("techsBoosted", list(&st.techs_boosted)),
        ("techProgress", named(&st.tech_progress)), ("currentResearch", opt(st.current_research)), ("civics", list(&st.civics_completed)),
        ("civicsInspired", list(&st.civics_inspired)), ("civicProgress", named(&st.civic_progress)), ("currentCivic", opt(st.current_civic)),
        ("government", opt(st.government)), ("policies", list(&st.policies_slotted)), ("unitsTrained", named(&st.units_trained)),
        ("goodyHutsReceived", named(&st.goody_huts)),
        ("luxuries", Value::Arr(st.luxuries.iter().map(|(c, r)| Value::obj(vec![("cityId", Value::from(*c)), ("resource", Value::from(*r))])).collect())),
        ("faithStock", religion.as_ref().map(|r| Value::from(r.0)).or(st.faith.map(Value::from)).unwrap_or(Value::Null)),
        ("pantheon", religion.as_ref().and_then(|r| r.1.clone()).map(Value::from).or(st.pantheon.map(Value::from)).unwrap_or(Value::Null)),
        // great person points banked / per turn per class; prophet points turn into faith once no prophet is on offer
        ("greatPersonPoints", Value::Obj(me.great_person_points.iter().map(|(c, b, r)| (c.to_string(), Value::obj(vec![("banked", Value::from(*b)), ("perTurn", Value::from(*r))]))).collect())),
        ("prophetOnOffer", Value::from(gp.iter().any(|x| x.class == "GREAT_PERSON_CLASS_PROPHET" && x.recruited_by.is_none()))),
        // great people this player recruited (individual, class, turn); the AI uses them at once
        ("greatPeopleRecruited", Value::Arr(gp.iter().filter(|x| x.recruited_by == Some(pid)).map(|x| Value::obj(vec![
            ("individual", s(x.individual)), ("cls", s(x.class)), ("turn", x.recruited_turn.map(Value::from).unwrap_or(Value::Null))])).collect())),
        // the first unclaimed individual of each class (who is on offer now)
        ("greatPeopleOnOffer", Value::Obj(on_offer)),
        ("prophetRecruitedTurn", gp.iter().find(|x| x.class == "GREAT_PERSON_CLASS_PROPHET" && x.recruited_by == Some(pid)).and_then(|x| x.recruited_turn).map(Value::from).unwrap_or(Value::Null)),
        ("governors", Value::Arr(governors)),
        ("envoys", Value::Arr(envoys)),
    ]);
    let city_yields = my_run.filter(|r| r.len() == my_count).map(|r| Value::Arr(r.clone())).unwrap_or(Value::Null);
    let units: Vec<Value> = g.units.iter().filter(|u| u.owner == pid).map(|u| Value::obj(vec![
        ("type", s(&u.type_name)), ("x", Value::from(u.x)), ("y", Value::from(u.y)), ("id", u.id.map(Value::from).unwrap_or(Value::Null)),
        ("charges", u.builder_charges.map(Value::from).unwrap_or(Value::Null)),
    ])).collect();
    // founded religions with their beliefs, and world-wide followers / cities following each
    let rels: Vec<Value> = {
        let rs = religions(&p, n);
        let mut agg: Vec<(i64, i64, Vec<Value>)> = rs.iter().map(|_| (0, 0, vec![])).collect();
        for c in &g.cities {
            let Some((majority, followers)) = city_religion(&p, n, name_at(c), c.population) else { continue };
            let own = ctx.own(c);
            for (k, r) in rs.iter().enumerate() {
                let f = followers.iter().find(|x| x.0 == r.0).map(|x| x.1).unwrap_or(0);
                agg[k].0 += f;
                let is_major = majority.as_deref() == Some(r.0.as_str());
                if is_major {
                    agg[k].1 += 1;
                }
                // founder beliefs count followers, following cities, their districts and wonders, at home and abroad
                if f > 0 || is_major {
                    let districts: Vec<Value> = own.iter().filter_map(|q| ctx.district_at.get(&(q.x, q.y))).filter(|dd| dd.1).map(|dd| s(dd.0)).collect();
                    let wonders = own.iter().filter_map(|q| q.owner.as_ref().and_then(|o| o.world_wonder.clone())).filter(|w| c.buildings.iter().any(|b| b == w)).count();
                    agg[k].2.push(Value::obj(vec![("name", s(&c.name)), ("playerId", Value::from(c.player_id)), ("followers", Value::from(f)),
                        ("majority", Value::from(is_major)), ("districts", Value::Arr(districts)), ("wonders", Value::from(wonders))]));
                }
            }
        }
        rs.into_iter().zip(agg).map(|((rel, founder, beliefs), (f, cities, list))| Value::obj(vec![
            ("religion", Value::from(rel)), ("founder", Value::from(founder)), ("beliefs", Value::Arr(beliefs.into_iter().map(Value::from).collect())),
            ("followers", Value::from(f)), ("cities", Value::from(cities)), ("cityList", Value::Arr(list)),
        ])).collect()
    };
    Value::obj(vec![
        ("file", s(file)), ("mapWidth", Value::from(g.map.width)), ("turn", Value::from(g.turn)), ("gameSpeed", s(&g.game_speed)), ("gameEra", game_era(&p)),
        ("player", player),
        ("cities", Value::Arr(cities)),
        ("cityYields", city_yields),
        ("units", Value::Arr(units)),
        ("religions", Value::Arr(rels)),
        // every city in the world (possible trade route destinations), in the route endpoint format
        ("worldCities", Value::Arr(g.cities.iter().filter(|c| c.player_id != pid).map(|c| ctx.route_city(c)).collect())),
        // world wonders finished anywhere (a plan can't build one another civ has finished)
        ("worldWonders", Value::Arr(g.cities.iter().flat_map(|c| c.buildings.iter().filter(|b| n.wonders.contains(**b))
            .map(move |b| Value::obj(vec![("building", s(b)), ("playerId", Value::from(c.player_id)), ("city", s(&c.name))]))).collect())),
        // routes starting or ending in this player's cities
        ("tradeRoutes", Value::Arr(trade_routes(&ctx, &p, pid))),
    ])
}
