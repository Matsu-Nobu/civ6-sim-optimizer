//! The plan's skeleton: the few choices that set an economy's course (the order of districts, the governors and
//! where they sit, the techs and civics aimed at, how far and where to expand), every other decision left to rules
//! (the planner's base policy). A skeleton is played by SkeletonChooser; the search changes skeletons, not the
//! thousands of single decisions.
use crate::model::*;
use crate::planner::{Chooser, Planner};
use crate::production::{item_kind, Kind};
use crate::rules::Rules;
use crate::sim::research_cost;
use crate::sym::Sym;
use std::collections::{BTreeSet, HashMap};

/// A governor title's use: appoint one to the n-th city (founding order), or promote one.
#[derive(Clone, Debug, PartialEq)]
pub enum GovPick {
    Appoint(Sym, usize),
    Promote(Sym, Sym),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Skeleton {
    /// the empire's first specialty districts, in the order they are built (afterwards the rules decide)
    pub districts: Vec<Sym>,
    /// techs (civics) aimed at in order; the prerequisites of the next one are taken cheapest first
    pub techs: Vec<Sym>,
    pub civics: Vec<Sym>,
    /// the governor titles' uses in order
    pub govs: Vec<GovPick>,
    /// city sites in founding order
    pub sites: Vec<XY>,
    /// cities to have (with the capital)
    pub n_cities: usize,
    /// a city trains settlers from this population on
    pub settler_pop: i32,
    /// builders kept per city
    pub builders_per_city: f64,
    /// cities training settlers at the same time (the most productive ones)
    pub settler_slots: usize,
    /// gold: buy the settler / builder a city is making, buy the best next tile (keeping `gold_reserve`)
    pub buy_units: bool,
    pub buy_tiles: bool,
    pub gold_reserve: f64,
}

/// Every tech (civic) `goal` needs, with itself.
fn prereq_closure(r: &Rules, goal: Sym, civic: bool) -> BTreeSet<Sym> {
    let (t, k, c) = if civic { ("CivicPrereqs", "Civic", "PrereqCivic") } else { ("TechnologyPrereqs", "Technology", "PrereqTech") };
    let mut out = BTreeSet::new();
    let mut todo = vec![goal];
    while let Some(x) = todo.pop() {
        if out.insert(x) {
            todo.extend(r.list_sym(t, k, x, c));
        }
    }
    out
}

pub struct SkeletonChooser<'a> {
    pub sk: &'a Skeleton,
    gov_at: usize,
    closures: HashMap<(Sym, bool), BTreeSet<Sym>>,
}

impl<'a> SkeletonChooser<'a> {
    pub fn new(sk: &'a Skeleton) -> Self {
        SkeletonChooser { sk, gov_at: 0, closures: HashMap::new() }
    }

    fn aim(&mut self, p: &Planner, st: &State, civic: bool, options: &[Choice]) -> Option<Choice> {
        let done = if civic { &st.acc.civics } else { &st.acc.techs };
        let goals = if civic { &self.sk.civics } else { &self.sk.techs };
        let era = p.sc.game_era(st.turn);
        let prog = if civic { &st.acc.civic_progress } else { &st.acc.tech_progress };
        for g in goals {
            if done.contains(g) {
                continue;
            }
            let cl = self.closures.entry((*g, civic)).or_insert_with(|| prereq_closure(p.r, *g, civic));
            let best = options.iter().filter(|o| o.sym().map(|s| cl.contains(&s)).unwrap_or(false)).min_by(|a, b| {
                let f = |c: &Choice| { let t = c.sym().unwrap(); research_cost(p.r, t, p.speed, era) - prog.get(&t).copied().unwrap_or(0.0) };
                f(a).partial_cmp(&f(b)).unwrap()
            });
            if let Some(b) = best {
                return Some(b.clone());
            }
        }
        None
    }

    /// Whether the city is among the `k` with the most production last turn.
    fn among_best(&self, st: &State, city: Sym, k: usize) -> bool {
        let mut v: Vec<(f64, Sym)> = st.city_output.iter().map(|(n, y)| (y[1], *n)).collect();
        if v.is_empty() {
            return true;
        }
        v.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        v.iter().take(k).any(|x| x.1 == city)
    }

    fn production(&self, p: &Planner, st: &State, city: Sym, options: &[Choice]) -> Option<Choice> {
        let has = |x: &str| options.contains(&Choice::S(crate::sym::sym(x)));
        let sk = self.sk;
        let n = st.cities.len();
        let pop = st.cacc(city).population;
        let making = |item: &str| st.cities.iter().filter(|c| c.current == Some(crate::sym::sym(item))).count();
        // builders first while short of them
        let want_b = (sk.builders_per_city * n as f64).ceil() as usize;
        if has("UNIT_BUILDER") && st.builders.len() + making("UNIT_BUILDER") < want_b {
            return Some(Choice::S(s!("UNIT_BUILDER")));
        }
        // settlers until the cities planned are founded or on their way
        if has("UNIT_SETTLER") && pop >= sk.settler_pop && n + st.settlers.len() + making("UNIT_SETTLER") < sk.n_cities
            && making("UNIT_SETTLER") < sk.settler_slots && self.among_best(st, city, sk.settler_slots) {
            return Some(Choice::S(s!("UNIT_SETTLER")));
        }
        let c = st.city(city)?;
        // a finished district's buildings, cheapest first
        let mut bs: Vec<(f64, Sym)> = options.iter().filter_map(|o| o.sym()).filter(|&b| item_kind(b) == Kind::Building && !p.r.is_wonder(b))
            .filter(|&b| p.r.lookup_sym("Buildings", "BuildingType", b, "PrereqDistrict")
                .map(|d| d != s!("DISTRICT_CITY_CENTER") && c.districts.iter().any(|x| x.complete && p.r.district_is(x.dtype, d))).unwrap_or(false))
            .map(|b| (p.r.cost(b), b)).collect();
        bs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        if let Some((_, b)) = bs.first() {
            return Some(Choice::S(*b));
        }
        // the empire's first specialty districts, in the skeleton's order: the next one goes to a city that can
        // build it; once the list is used up the rules decide
        let placed = st.cities.iter().flat_map(|c| c.districts.iter()).filter(|d| p.r.district_requires_population(d.dtype)).count();
        let list: Vec<Sym> = sk.districts.iter().copied().filter(|d| p.r.district_requires_population(*d)).collect();
        if let Some(d) = list.get(placed) {
            if options.contains(&Choice::S(*d)) {
                return Some(Choice::S(*d));
            }
        }
        None
    }
}

impl Chooser for SkeletonChooser<'_> {
    fn choose(&mut self, p: &Planner, st: &State, key: &Key, options: &[Choice]) -> Option<Choice> {
        let k = key.kind;
        let pick = if k == s!("research") || k == s!("civic") {
            self.aim(p, st, k == s!("civic"), options)
        } else if k == s!("settle") {
            self.sk.sites.iter().map(|xy| Choice::Xy(xy.0, xy.1)).find(|c| options.contains(c))
        } else if k == s!("governor") {
            let mut found = None;
            for i in self.gov_at..self.sk.govs.len() {
                let c = match &self.sk.govs[i] {
                    GovPick::Appoint(g, ci) => st.cities.get(*ci).map(|c| Choice::Appoint(*g, c.name)),
                    GovPick::Promote(g, pr) => Some(Choice::Promote(*g, *pr)),
                };
                if let Some(c) = c.filter(|c| options.contains(c)) {
                    self.gov_at = i + 1;
                    found = Some(c);
                    break;
                }
            }
            found
        } else if k == s!("buy") {
            let gold = st.acc.gold - self.sk.gold_reserve;
            let unit = options.iter().filter(|o| self.sk.buy_units && matches!(o, Choice::BuyItem(c, it, g, cur)
                if *cur == s!("YIELD_GOLD") && (*it == s!("UNIT_SETTLER") || *it == s!("UNIT_BUILDER")) && f64::from_bits(*g) <= gold
                    && st.city(*c).map(|x| x.current == Some(*it)).unwrap_or(false))).min_by(|a, b| {
                let g = |o: &Choice| match o { Choice::BuyItem(_, _, g, _) => f64::from_bits(*g), _ => 0.0 };
                g(a).partial_cmp(&g(b)).unwrap()
            }).cloned();
            let tile = || options.iter().filter(|o| self.sk.buy_tiles && matches!(o, Choice::BuyTile(_, g, _, _) if f64::from_bits(*g) <= gold)).min_by(|a, b| {
                let g = |o: &Choice| match o { Choice::BuyTile(_, g, _, _) => f64::from_bits(*g), _ => 0.0 };
                g(a).partial_cmp(&g(b)).unwrap()
            }).cloned();
            unit.or_else(tile).or(Some(Choice::Nothing))
        } else if k == s!("prod") {
            match key.ent {
                Ent::S(city) => self.production(p, st, city, options).or_else(|| {
                    // the rest by the base policy, without settlers and builders beyond the skeleton's
                    let n = st.cities.len();
                    let making = |item: Sym| st.cities.iter().filter(|c| c.current == Some(item)).count();
                    let enough_s = n + st.settlers.len() + making(s!("UNIT_SETTLER")) >= self.sk.n_cities || st.cacc(city).population < self.sk.settler_pop
                        || making(s!("UNIT_SETTLER")) >= self.sk.settler_slots || !self.among_best(st, city, self.sk.settler_slots);
                    let enough_b = st.builders.len() + making(s!("UNIT_BUILDER")) >= (self.sk.builders_per_city * n as f64).ceil() as usize;
                    let rest: Vec<Choice> = options.iter().filter(|o| !(enough_s && **o == Choice::S(s!("UNIT_SETTLER"))) && !(enough_b && **o == Choice::S(s!("UNIT_BUILDER")))).cloned().collect();
                    if rest.is_empty() { None } else { p.base(st, key, &rest) }
                }),
                _ => None,
            }
        } else {
            None
        };
        pick.or_else(|| p.base(st, key, options))
    }
}

/// The recorded player's skeleton: districts in the order they were placed, techs and civics in the order they
/// were completed, the governor titles as used, the cities' sites (up to `last`).
pub fn from_record(rec: &crate::record::Record, last: i64) -> Skeleton {
    let mut districts: Vec<Sym> = vec![];
    let mut seen: std::collections::HashSet<(Sym, XY)> = std::collections::HashSet::new();
    let mut govs: Vec<GovPick> = vec![];
    let mut known: HashMap<Sym, Vec<Sym>> = HashMap::new();
    let order: Vec<Sym> = rec.sites.iter().map(|s| s.1).collect();
    let mut capital: Option<Sym> = None;
    for (t, r) in rec.turns.iter() {
        if *t > last {
            break;
        }
        if capital.is_none() {
            capital = r.cities.first().map(|c| c.name);
        }
        for c in &r.cities {
            for d in &c.districts {
                if d.0 != s!("DISTRICT_CITY_CENTER") && d.0 != s!("DISTRICT_WONDER") && seen.insert((c.name, d.1)) {
                    districts.push(d.0);
                }
            }
        }
        for (g, city, prs) in &r.governors {
            if !known.contains_key(g) {
                let ci = city.and_then(|cn| if Some(cn) == capital { Some(0) } else { order.iter().position(|x| *x == cn).map(|i| i + 1) });
                govs.push(GovPick::Appoint(*g, ci.unwrap_or(0)));
                known.insert(*g, prs.clone());
                continue;
            }
            for pr in prs {
                if !known[g].contains(pr) {
                    govs.push(GovPick::Promote(*g, *pr));
                    known.get_mut(g).unwrap().push(*pr);
                }
            }
        }
    }
    let sites: Vec<XY> = rec.sites.iter().filter(|s| s.0 <= last).map(|s| s.2).collect();
    Skeleton {
        districts,
        techs: rec.completion_order(false),
        civics: rec.completion_order(true),
        govs,
        n_cities: sites.len() + 1,
        sites,
        settler_pop: 2,
        builders_per_city: 1.0,
        settler_slots: 1,
        buy_units: true,
        buy_tiles: true,
        gold_reserve: 0.0,
    }
}

/// What a played game offered at its decisions: the pools mutations draw from.
#[derive(Default, Clone)]
pub struct Pools {
    pub districts: Vec<Sym>,
    pub techs: Vec<Sym>,
    pub civics: Vec<Sym>,
    pub govs: Vec<GovPick>,
    pub sites: Vec<XY>,
}

/// Records the options of every decision while another chooser plays.
pub struct Watch<'c> {
    pub inner: &'c mut dyn Chooser,
    pub pools: Pools,
}

impl Chooser for Watch<'_> {
    fn choose(&mut self, p: &Planner, st: &State, key: &Key, options: &[Choice]) -> Option<Choice> {
        let k = key.kind;
        let pools = &mut self.pools;
        for o in options {
            match o {
                Choice::Xy(x, y) if k == s!("settle") && !pools.sites.contains(&(*x, *y)) => pools.sites.push((*x, *y)),
                Choice::S(s) if k == s!("prod") && item_kind(*s) == Kind::District && !pools.districts.contains(s) => pools.districts.push(*s),
                Choice::S(s) if k == s!("research") && !pools.techs.contains(s) => pools.techs.push(*s),
                Choice::S(s) if k == s!("civic") && !pools.civics.contains(s) => pools.civics.push(*s),
                Choice::Appoint(g, c) => {
                    let ci = st.cities.iter().position(|x| x.name == *c).unwrap_or(0);
                    let gp = GovPick::Appoint(*g, ci);
                    if !pools.govs.contains(&gp) {
                        pools.govs.push(gp);
                    }
                }
                Choice::Promote(g, pr) => {
                    let gp = GovPick::Promote(*g, *pr);
                    if !pools.govs.contains(&gp) {
                        pools.govs.push(gp);
                    }
                }
                _ => {}
            }
        }
        self.inner.choose(p, st, key, options)
    }
}

impl Pools {
    pub fn merge(&mut self, o: &Pools) {
        fn m<T: Clone + PartialEq>(a: &mut Vec<T>, b: &[T]) {
            for x in b {
                if !a.contains(x) {
                    a.push(x.clone());
                }
            }
        }
        m(&mut self.districts, &o.districts);
        m(&mut self.techs, &o.techs);
        m(&mut self.civics, &o.civics);
        m(&mut self.govs, &o.govs);
        m(&mut self.sites, &o.sites);
    }
}

/// One random change of a skeleton (or a few, `n`).
pub fn mutate(sk: &Skeleton, pools: &Pools, rng: &mut crate::opt::Rng, n: usize) -> Skeleton {
    let mut s = sk.clone();
    let pick = |rng: &mut crate::opt::Rng, len: usize| (rng.next_u64() % len.max(1) as u64) as usize;
    for _ in 0..n {
        match rng.next_u64() % 12 {
            // the district order: move one, add one, drop one
            0 | 1 if s.districts.len() >= 2 => {
                let i = pick(rng, s.districts.len());
                let x = s.districts.remove(i);
                let j = pick(rng, s.districts.len() + 1);
                s.districts.insert(j, x);
            }
            2 if !pools.districts.is_empty() => {
                if rng.next_u64() % 2 == 0 || s.districts.len() < 2 {
                    let d = pools.districts[pick(rng, pools.districts.len())];
                    let j = pick(rng, s.districts.len() + 1);
                    s.districts.insert(j, d);
                } else {
                    let i = pick(rng, s.districts.len());
                    s.districts.remove(i);
                }
            }
            // techs / civics aimed at: move one earlier or later, or put a new one in
            3 | 4 | 5 | 6 => {
                let civic = rng.next_u64() % 2 == 0;
                let (list, pool) = if civic { (&mut s.civics, &pools.civics) } else { (&mut s.techs, &pools.techs) };
                if rng.next_u64() % 3 == 0 && !pool.is_empty() {
                    let x = pool[pick(rng, pool.len())];
                    list.retain(|y| *y != x);
                    let j = pick(rng, list.len().min(15) + 1);
                    list.insert(j, x);
                } else if list.len() >= 2 {
                    let i = pick(rng, list.len().min(25));
                    let x = list.remove(i);
                    let j = pick(rng, list.len().min(25) + 1);
                    list.insert(j, x);
                }
            }
            // governors: another use for one title, or two swapped
            7 | 8 if !pools.govs.is_empty() => {
                let g = pools.govs[pick(rng, pools.govs.len())].clone();
                if s.govs.is_empty() || rng.next_u64() % 3 == 0 {
                    let j = pick(rng, s.govs.len() + 1);
                    s.govs.insert(j, g);
                } else {
                    let i = pick(rng, s.govs.len());
                    s.govs[i] = g;
                }
            }
            // a site: another one, or two swapped
            9 if !pools.sites.is_empty() && !s.sites.is_empty() => {
                let i = pick(rng, s.sites.len());
                if rng.next_u64() % 2 == 0 {
                    let x = pools.sites[pick(rng, pools.sites.len())];
                    if !s.sites.contains(&x) {
                        s.sites[i] = x;
                    }
                } else {
                    let j = pick(rng, s.sites.len());
                    s.sites.swap(i, j);
                }
            }
            // how far and how fast to expand
            10 => {
                if rng.next_u64() % 2 == 0 {
                    s.n_cities = (s.n_cities as i64 + if rng.next_u64() % 2 == 0 { 1 } else { -1 }).clamp(1, 20) as usize;
                    while s.sites.len() + 1 < s.n_cities && !pools.sites.is_empty() {
                        let x = pools.sites[pick(rng, pools.sites.len())];
                        if !s.sites.contains(&x) {
                            s.sites.push(x);
                        } else {
                            break;
                        }
                    }
                } else {
                    s.settler_pop = (s.settler_pop + if rng.next_u64() % 2 == 0 { 1 } else { -1 }).clamp(1, 8);
                }
            }
            11 => match rng.next_u64() % 5 {
                0 => s.settler_slots = (s.settler_slots as i64 + if rng.next_u64() % 2 == 0 { 1 } else { -1 }).clamp(1, 6) as usize,
                1 => s.buy_units = !s.buy_units,
                2 => s.buy_tiles = !s.buy_tiles,
                3 => s.gold_reserve = (s.gold_reserve + if rng.next_u64() % 2 == 0 { 50.0 } else { -50.0 }).clamp(0.0, 500.0),
                _ => s.builders_per_city = (s.builders_per_city + if rng.next_u64() % 2 == 0 { 0.25 } else { -0.25 }).clamp(0.25, 3.0),
            },
            _ => {}
        }
    }
    s
}

pub fn to_json(sk: &Skeleton) -> crate::json::Value {
    use crate::json::Value;
    let syms = |v: &[Sym]| Value::Arr(v.iter().map(|x| Value::from(x.as_string())).collect());
    Value::obj(vec![
        ("districts", syms(&sk.districts)),
        ("techs", syms(&sk.techs)),
        ("civics", syms(&sk.civics)),
        ("govs", Value::Arr(sk.govs.iter().map(|g| match g {
            GovPick::Appoint(g, c) => Value::Arr(vec![Value::from("appoint"), Value::from(g.as_string()), Value::from(*c as i64)]),
            GovPick::Promote(g, p) => Value::Arr(vec![Value::from("promote"), Value::from(g.as_string()), Value::from(p.as_string())]),
        }).collect())),
        ("sites", Value::Arr(sk.sites.iter().map(|xy| Value::Arr(vec![Value::from(xy.0), Value::from(xy.1)])).collect())),
        ("n_cities", Value::from(sk.n_cities as i64)),
        ("settler_pop", Value::from(sk.settler_pop as i64)),
        ("builders_per_city", Value::from(sk.builders_per_city)),
        ("settler_slots", Value::from(sk.settler_slots as i64)),
        ("buy_units", Value::from(sk.buy_units)),
        ("buy_tiles", Value::from(sk.buy_tiles)),
        ("gold_reserve", Value::from(sk.gold_reserve)),
    ])
}
