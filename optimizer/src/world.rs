//! The planner's world (civ6sim/world.py): the wrapping map, movement, borders, citizens, settling.
use crate::engine::{ss, Engine};
use crate::model::*;
use crate::rules::Rules;
use crate::sym::Sym;
use crate::yields::{tile_yields, Plots};
use crate::json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};

pub const WORKABLE_RADIUS: i32 = 3;
const BORDER_COST_SCALE: f64 = 0.5;

pub struct Map {
    pub width: i32,
    pub height: i32,
    pub plots: HashMap<XY, Plot>,
    lake: HashMap<XY, bool>,
}

const RIVER_BIT_OF_NEIGHBOUR: [u8; 6] = [1, 4, 3, 2, 5, 0]; // neighbour index -> bit (from {5: 0, 0: 1, 3: 2, 2: 3, 1: 4, 4: 5})

impl Map {
    pub fn load(v: &Value) -> Map {
        crate::geo::set_map_width(v["width"].as_i64().unwrap() as i32);
        let mut m = Map {
            width: v["width"].as_i64().unwrap() as i32,
            height: v["height"].as_i64().unwrap() as i32,
            plots: v["plots"].as_array().unwrap().iter().map(|p| { let q = plot_from(p); (q.xy(), q) }).collect(),
            lake: HashMap::new(),
        };
        let keys: Vec<XY> = m.plots.keys().copied().collect();
        for xy in keys {
            if !m.lake.contains_key(&xy) {
                m.compute_lake(xy);
            }
        }
        // the plots carry it too (fresh water and housing read plot.lake, as on the save's city plots)
        for (xy, p) in m.plots.iter_mut() {
            if m.lake.get(xy).copied().unwrap_or(false) {
                p.lake = true;
            }
        }
        m
    }

    fn dirs(y: i32) -> [(i32, i32); 6] {
        if y & 1 == 1 { [(1, 0), (-1, 0), (0, -1), (1, -1), (0, 1), (1, 1)] } else { [(1, 0), (-1, 0), (-1, -1), (0, -1), (-1, 1), (0, 1)] }
    }

    pub fn neighbours(&self, x: i32, y: i32) -> Vec<XY> {
        Map::dirs(y).iter().filter(|(_, dy)| 0 <= y + dy && y + dy < self.height).map(|(dx, dy)| ((x + dx).rem_euclid(self.width), y + dy)).collect()
    }

    pub fn river_between(&self, a: XY, b: XY) -> bool {
        let Some(p) = self.plots.get(&a) else { return false };
        if p.river_edges == 0 {
            return false;
        }
        for (j, (dx, dy)) in Map::dirs(a.1).iter().enumerate() {
            let ny = a.1 + dy;
            if !(0 <= ny && ny < self.height) {
                continue;
            }
            if ((a.0 + dx).rem_euclid(self.width), ny) == b {
                return p.river_edges >> RIVER_BIT_OF_NEIGHBOUR[j] & 1 == 1;
            }
        }
        false
    }

    fn cube(x: i32, y: i32) -> (i32, i32) {
        (x - (y - (y & 1)).div_euclid(2), y)
    }

    pub fn distance(&self, a: XY, b: XY) -> i32 {
        let mut best = i32::MAX;
        for wrap in [-self.width, 0, self.width] {
            let (q1, r1) = Map::cube(a.0, a.1);
            let (q2, r2) = Map::cube(b.0 + wrap, b.1);
            let (dq, dr) = (q1 - q2, r1 - r2);
            let d = (dq.abs() + dr.abs() + (dq + dr).abs()) / 2;
            best = best.min(d);
        }
        best
    }

    /// Plots within `radius` steps, sorted.
    pub fn within(&self, center: XY, radius: i32) -> Vec<XY> {
        let mut seen: HashSet<XY> = HashSet::from([center]);
        let mut frontier = vec![center];
        for _ in 0..radius {
            let mut nxt = vec![];
            for xy in frontier {
                for n in self.neighbours(xy.0, xy.1) {
                    if seen.insert(n) {
                        nxt.push(n);
                    }
                }
            }
            frontier = nxt;
        }
        let mut v: Vec<XY> = seen.into_iter().collect();
        v.sort();
        v
    }

    fn is_waterish(&self, xy: XY) -> bool {
        self.plots.get(&xy).map(|p| p.terrain.ends_with("COAST") || p.terrain.ends_with("OCEAN")).unwrap_or(false)
    }

    fn compute_lake(&mut self, xy: XY) {
        if !self.is_waterish(xy) {
            self.lake.insert(xy, false);
            return;
        }
        let mut seen: HashSet<XY> = HashSet::from([xy]);
        let mut frontier = vec![xy];
        while !frontier.is_empty() && seen.len() <= 9 {
            let mut nxt = vec![];
            for a in frontier {
                for n in self.neighbours(a.0, a.1) {
                    if !seen.contains(&n) && self.is_waterish(n) {
                        seen.insert(n);
                        nxt.push(n);
                    }
                }
            }
            frontier = nxt;
        }
        let small = seen.len() <= 9;
        for a in seen {
            self.lake.insert(a, small);
        }
    }

    pub fn is_lake(&self, xy: XY) -> bool {
        *self.lake.get(&xy).unwrap_or(&false)
    }

    pub fn is_land(&self, xy: XY) -> bool {
        let t = self.plots[&xy].terrain;
        !(t.ends_with("OCEAN") || t.ends_with("COAST") || t.ends_with("MOUNTAIN"))
    }
}

/// Movement cost of one step (world.move_cost), or None if the step is not possible.
pub static RIVER_MIN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// civ6opt movecheck --any-points: a unit with any points left may enter a plot that costs more (to test the rule)
pub static ANY_POINTS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn move_cost(r: &Rules, m: &Map, a: XY, b: XY, left: f64, full: f64, road_type: Sym, cities: &HashSet<XY>) -> Option<f64> {
    let p = m.plots.get(&b)?;
    if p.terrain.ends_with("MOUNTAIN") || p.terrain == s!("TERRAIN_COAST") || p.terrain == s!("TERRAIN_OCEAN") {
        return None;
    }
    if let Some(f) = p.feature {
        if r.lookup_truthy("Features", "FeatureType", f, "Impassable") {
            return None;
        }
    }
    let pa = &m.plots[&a];
    let road = pa.route.is_some() && p.route.is_some();
    // a road between two plots is the poorer of their roads (the save's route index is the row of Routes); a plot
    // whose road type is not known has the era's (road_type)
    let rt = match (pa.route, p.route) {
        (Some(x), Some(y)) if x >= 0 && y >= 0 => r.tab("Routes").v(x.min(y) as u32, "RouteType").sym().unwrap_or(road_type),
        _ => road_type,
    };
    let road_cost = r.lookup_f("Routes", "RouteType", rt, "MovementCost");
    let bridges = r.lookup_truthy("Routes", "RouteType", rt, "SupportsBridges");
    // crossing a river costs max(3, the points left): a unit with fewer than 3 points crosses only with full points
    // (the user's rule), and the crossing ends its moves; RIVER_MIN (civ6opt movecheck --river-min) tries min(3, left)
    let cost = if m.river_between(a, b) && !(road && bridges) && !cities.contains(&a) && !cities.contains(&b) {
        if RIVER_MIN.load(std::sync::atomic::Ordering::Relaxed) { 3f64.min(left) } else { 3f64.max(left) }
    } else if road {
        road_cost
    } else if cities.contains(&b) {
        r.param("CITY_MOVEMENT_COST")
    } else {
        let mut c = r.lookup_f("Terrains", "TerrainType", p.terrain, "MovementCost");
        if let Some(f) = p.feature {
            c += r.lookup_f("Features", "FeatureType", f, "MovementChange");
        }
        c
    };
    if cost <= left || left == full || (ANY_POINTS.load(std::sync::atomic::Ordering::Relaxed) && left > 0.0) {
        Some(cost.min(left))
    } else {
        None
    }
}

/// Turns until a unit standing on `start` with full points can act on `goal` (0 = this turn).
pub fn travel_turns(r: &Rules, m: &Map, start: XY, goal: XY, moves: f64, road_type: Sym, cities: &HashSet<XY>) -> Option<i64> {
    travel_search(r, m, start, goal, moves, road_type, cities, false).map(|x| x.0)
}

/// travel_turns and the way: each plot after the start with the turn (0: the first) the unit enters it.
pub fn travel_path(r: &Rules, m: &Map, start: XY, goal: XY, moves: f64, road_type: Sym, cities: &HashSet<XY>) -> Option<(i64, Vec<(XY, i64)>)> {
    travel_search(r, m, start, goal, moves, road_type, cities, true)
}

fn travel_search(r: &Rules, m: &Map, start: XY, goal: XY, moves: f64, road_type: Sym, cities: &HashSet<XY>, keep_path: bool) -> Option<(i64, Vec<(XY, i64)>)> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    if start == goal {
        return Some((0, vec![]));
    }
    // states ordered by (turn, -points left); points kept as integer quarters for the heap
    let q = |x: f64| (x * 4.0).round() as i64;
    let mut best: HashMap<XY, (i64, i64)> = HashMap::from([(start, (0, -q(moves)))]);
    let mut came_from: HashMap<XY, XY> = HashMap::new();
    let mut heap = BinaryHeap::from([Reverse((0i64, -q(moves), start))]);
    while let Some(Reverse((t, neg_left, at))) = heap.pop() {
        let left = -(neg_left as f64) / 4.0;
        if best.get(&at).map(|b| *b < (t, neg_left)).unwrap_or(false) {
            continue;
        }
        if at == goal {
            let turns = if left > 0.0 { t } else { t + 1 };
            let mut path = vec![];
            if keep_path {
                let mut p = goal;
                while p != start {
                    path.push((p, best[&p].0));
                    p = came_from[&p];
                }
                path.reverse();
            }
            return Some((turns, path));
        }
        for n in m.neighbours(at.0, at.1) {
            for (tt, ll) in [(t, left), (t + 1, moves)] {
                if ll <= 0.0 {
                    continue;
                }
                let Some(c) = move_cost(r, m, at, n, ll, moves, road_type, cities) else { continue };
                let state = (tt, -q(ll - c));
                if tt <= 30 && state < *best.get(&n).unwrap_or(&(99, 0)) {
                    best.insert(n, state);
                    if keep_path {
                        came_from.insert(n, at);
                    }
                    heap.push(Reverse((tt, -q(ll - c), n)));
                }
                break;
            }
        }
    }
    None
}

pub type Weights = [f64; 6];
pub const DEFAULT_WEIGHTS: Weights = [1.0, 1.0, 0.5, 0.8, 0.6, 0.2];

pub fn weighted(w: &Weights, y: &[(Sym, f64)]) -> f64 {
    y.iter().map(|(k, v)| crate::rules::yidx(*k).map(|i| w[i]).unwrap_or(0.0) * v).sum()
}

pub fn plot_value(r: &Rules, p: &Plot, techs: &BTreeSet<Sym>, civics: &BTreeSet<Sym>, w: &Weights) -> f64 {
    weighted(w, &tile_yields(r, p, techs, civics, false, &[], None, None, None))
}

pub struct World<'a> {
    pub r: &'a Rules,
    pub map: Map,
    pub foreign: HashSet<XY>,
    /// site_score's value of a site's surroundings (map plots only) by (site, techs, civics)
    site_memo: std::cell::RefCell<HashMap<(XY, u64), f64>>,
}

impl<'a> World<'a> {
    pub fn new(r: &'a Rules, map: Map, player_id: i64) -> World<'a> {
        let foreign = map.plots.iter().filter(|(_, p)| p.owner.is_some() && p.owner != Some(player_id)).map(|(xy, _)| *xy).collect();
        World { r, map, foreign, site_memo: Default::default() }
    }

    pub fn border_cost(&self, n: i64) -> f64 {
        let r = self.r;
        (BORDER_COST_SCALE * (r.param("CULTURE_COST_FIRST_PLOT") + (r.param("CULTURE_COST_LATER_PLOT_MULTIPLIER") * n as f64).powf(r.param("CULTURE_COST_LATER_PLOT_EXPONENT")))).floor()
    }

    pub fn owned_by_us(cities: &[City]) -> HashSet<XY> {
        cities.iter().flat_map(|c| c.owned.iter().map(|p| p.xy())).collect()
    }

    /// The tile a border takes next: best value, then closest; ties: the smallest (x, y).
    pub fn next_border_tile(&self, ci: usize, cities: &[City], techs: &BTreeSet<Sym>, civics: &BTreeSet<Sym>, w: &Weights) -> Option<XY> {
        self.border_tiles(ci, cities, techs, civics, w, 1).into_iter().next()
    }

    /// The `k` best plots a city could take next (adjacent to it, in its workable radius, nobody's), best first.
    pub fn border_tiles(&self, ci: usize, cities: &[City], techs: &BTreeSet<Sym>, civics: &BTreeSet<Sym>, w: &Weights, k: usize) -> Vec<XY> {
        let mine = World::owned_by_us(cities);
        let city = &cities[ci];
        let mut cands: Vec<XY> = vec![];
        for p in &city.owned {
            for n in self.map.neighbours(p.x, p.y) {
                if !mine.contains(&n) && !self.foreign.contains(&n) && self.map.distance(city.center, n) <= WORKABLE_RADIUS && !cands.contains(&n) {
                    cands.push(n);
                }
            }
        }
        cands.sort();
        // by value, then nearness, then the plot (the first of equals as before)
        let mut scored: Vec<(f64, i32, usize, XY)> = cands.iter().enumerate()
            .map(|(i, &xy)| (plot_value(self.r, &self.map.plots[&xy], techs, civics, w), -self.map.distance(city.center, xy), i, xy)).collect();
        scored.sort_by(|a, b| (b.0, b.1).partial_cmp(&(a.0, a.1)).unwrap().then(a.2.cmp(&b.2)));
        scored.into_iter().take(k).map(|x| x.3).collect()
    }

    pub fn workable(&self, xy: XY, center: XY, e: &Engine) -> bool {
        if self.map.distance(xy, center) > WORKABLE_RADIUS {
            return false;
        }
        let p = &self.map.plots[&xy];
        if p.terrain.ends_with("MOUNTAIN") {
            let ok: Vec<Sym> = e.player_effects(s!("EFFECT_ADJUST_PLAYER_TERRAIN_WORK_IMPASSABLE_MODIFIER")).iter().filter_map(|x| x.args.get("TerrainType")).collect();
            if !ok.contains(&p.terrain) {
                return false;
            }
        }
        if let Some(f) = p.feature {
            if self.r.lookup_truthy("Features", "FeatureType", f, "Impassable") {
                return false;
            }
        }
        true
    }

    /// Work the best tiles by their full yields; keep at least `min_surplus` food if possible.
    pub fn assign_citizens(&self, city: &mut City, e: &Engine, w: &Weights, min_surplus: f64) {
        let _g = crate::prof::start(2);
        let r = self.r;
        let center = city.center;
        let dtiles: Vec<XY> = city.districts.iter().map(|d| (d.x, d.y)).collect();
        let mods = e.plot_effects.get(&city.name).cloned().unwrap_or_default();
        let pxy: &Plots = &e.plots;
        let amods = crate::yields::appeal_modifiers(e, city.name);
        let cc = city.clone();
        let ty = |p: &Plot, c: bool| tile_yields(r, p, &e.env.techs, &e.env.civics, c, &mods, Some(pxy), Some(&cc), Some(&amods));
        let mut scored: Vec<(f64, f64, usize)> = vec![];
        for (i, p) in city.owned.iter().enumerate() {
            if p.xy() == center || dtiles.contains(&p.xy()) || !self.workable(p.xy(), center, e) {
                continue;
            }
            let y = ty(p, false);
            scored.push((weighted(w, &y), crate::yields::get_y(&y, s!("YIELD_FOOD")), i));
        }
        scored.sort_by(|a, b| (-a.0, -a.1).partial_cmp(&(-b.0, -b.1)).unwrap());
        let pop = city.population.max(0) as usize;
        let food_of: HashMap<usize, f64> = scored.iter().map(|t| (t.2, t.1)).collect();
        let mut chosen: Vec<usize> = scored.iter().take(pop).map(|t| t.2).collect();
        let ci = city.owned.iter().position(|p| p.xy() == center).unwrap();
        let center_food = crate::yields::get_y(&ty(&city.owned[ci], true), s!("YIELD_FOOD"));
        let surplus = |sel: &[usize]| center_food + sel.iter().map(|i| food_of[i]).sum::<f64>() - 2.0 * city.population as f64;
        let mut spare: Vec<usize> = scored.iter().skip(pop).map(|t| t.2).collect();
        spare.sort_by(|a, b| (-food_of[a]).partial_cmp(&(-food_of[b])).unwrap());
        let mut spare = std::collections::VecDeque::from(spare);
        while !spare.is_empty() && !chosen.is_empty() && surplus(&chosen) < min_surplus {
            let wi = (0..chosen.len()).min_by(|&a, &b| food_of[&chosen[a]].partial_cmp(&food_of[&chosen[b]]).unwrap()).unwrap();
            let better = spare.pop_front().unwrap();
            if food_of[&better] <= food_of[&chosen[wi]] {
                break;
            }
            chosen[wi] = better;
        }
        let mut worked = vec![city.owned[ci].clone()];
        worked[0].workers = 1;
        for i in chosen {
            let mut p = city.owned[i].clone();
            p.workers = 1;
            worked.push(p);
        }
        city.worked = worked;
    }

    pub fn site_score(&self, site: XY, cities: &[City], techs: &BTreeSet<Sym>, civics: &BTreeSet<Sym>) -> f64 {
        if !self.map.is_land(site) || self.foreign.contains(&site) {
            return 0.0;
        }
        if cities.iter().any(|c| self.map.distance(site, c.center) < 4) {
            return 0.0;
        }
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            techs.hash(&mut h);
            civics.hash(&mut h);
            (site, h.finish())
        };
        if let Some(v) = self.site_memo.borrow().get(&key) {
            return *v;
        }
        let mut vals: Vec<f64> = self.map.within(site, 2).into_iter().filter(|&xy| xy != site && self.map.is_land(xy))
            .map(|xy| plot_value(self.r, &self.map.plots[&xy], techs, civics, &DEFAULT_WEIGHTS)).collect();
        vals.sort_by(|a, b| b.partial_cmp(a).unwrap());
        let fresh = if self.map.plots[&site].river { 2.0 } else { 0.0 };
        let v = vals.iter().take(6).sum::<f64>() + fresh;
        let mut m = self.site_memo.borrow_mut();
        if m.len() > 200_000 {
            m.clear();
        }
        m.insert(key, v);
        v
    }
}
