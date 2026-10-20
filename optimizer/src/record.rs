//! Replaying the recorded game: the player's choices read turn by turn from the saves (problem "rows"), and the
//! state the planner takes from them while replaying (Planner::rec): the items each city is producing (switching
//! keeps progress), where its districts stand, the tiles its citizens work, the tiles it owns, gold and faith.
//!
//! The saves are snapshots, not a log, so the choices are read from consecutive snapshots:
//!   research / civic   the one completed during the turn, else the one whose progress grew by about the turn's
//!                      science (culture) (a boost grows another by a lump); the save's "current research" is the
//!                      queue's target, not the tech being researched
//!   production         the city's current item at the turn
//!   policies, government, governors, pantheon, great people   the next turn's snapshot
//!   builder actions    improvements that appeared and features / resources that vanished from a plot without a
//!                      district (chops, harvests), on their turn
//!   purchases          units trained beyond what the cities finished; buildings that appeared without being the
//!                      city's item
//!   trade routes       the routes' origin, destination and start turn
//!   city sites         the recorded cities, in founding order
use crate::json::Value;
use crate::model::*;
use crate::planner::{Chooser, Planner};
use crate::sym::{sym, Sym};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Debug, Default)]
pub struct RecCity {
    pub name: Sym,
    pub center: XY,
    pub pop: i64,
    pub food: f64,
    /// the city's food and production this turn
    pub food_yield: f64,
    pub prod_yield: f64,
    pub current: Option<Sym>,
    /// the current item's progress
    pub progress: f64,
    pub worked: Vec<XY>,
    pub owned: Vec<XY>,
    pub buildings: Vec<Sym>,
    pub districts: Vec<(Sym, XY, bool)>,
}

#[derive(Clone, Debug, Default)]
pub struct RecTurn {
    pub techs: Vec<Sym>,
    pub civics: Vec<Sym>,
    pub tech_progress: HashMap<Sym, f64>,
    pub civic_progress: HashMap<Sym, f64>,
    pub science: f64,
    pub culture: f64,
    /// food + production of the cities this turn (the objective)
    pub food_prod: f64,
    /// the player's food + production as the save records it: what the previous turn produced (the city yields
    /// above are those of this save's state; the player may reassign citizens during a turn)
    pub player_food_prod: f64,
    pub gov: Option<Sym>,
    pub policies: Vec<Sym>,
    /// (governor, city name, promotions)
    pub governors: Vec<(Sym, Option<Sym>, Vec<Sym>)>,
    pub pantheon: Option<Sym>,
    pub gold: f64,
    pub faith: f64,
    /// gold and faith the turn brought in (to tell spending from income)
    pub gold_income: f64,
    pub faith_income: f64,
    pub units_trained: HashMap<Sym, i64>,
    pub great_people: Vec<Sym>,
    pub dedications: Vec<Sym>,
    /// techs boosted and civics inspired so far
    pub boosted: Vec<Sym>,
    pub cities: Vec<RecCity>,
}

impl RecTurn {
    pub fn city(&self, name: Sym) -> Option<&RecCity> {
        self.cities.iter().find(|c| c.name == name)
    }
}

/// A builder action of the recorded game: (turn, "improve" | "harvest", plot, improvement | "feature" | "resource")
pub type Act = (i64, Sym, XY, Sym);

pub struct Record {
    pub turns: BTreeMap<i64, RecTurn>,
    /// research / civic being worked on during each turn
    pub research: HashMap<i64, Sym>,
    pub civic: HashMap<i64, Sym>,
    pub acts: Vec<Act>,
    /// (turn, city, item)
    pub purchases: Vec<(i64, Sym, Sym)>,
    /// (turn started, origin, destination)
    pub routes: Vec<(i64, Sym, Sym)>,
    /// (turn founded, name, site), in founding order
    pub sites: Vec<(i64, Sym, XY)>,
    /// city centre -> [(turn, plot)] the city got after its founding turn
    pub tiles: HashMap<XY, Vec<(i64, XY)>>,
    /// city centre -> the plots it owned on its first recorded turn
    pub first_owned: HashMap<XY, Vec<XY>>,
}

fn xy_of(v: &Value) -> XY {
    (v["x"].as_i64().unwrap_or(0) as i32, v["y"].as_i64().unwrap_or(0) as i32)
}

fn osym(v: &Value) -> Option<Sym> {
    v.as_str().filter(|s| !s.is_empty()).map(sym)
}

fn syms(v: &Value) -> Vec<Sym> {
    v.as_array().map(|a| a.iter().filter_map(osym).collect()).unwrap_or_default()
}

fn fmap(v: &Value) -> HashMap<Sym, f64> {
    v.as_object().map(|o| o.iter().map(|(k, x)| (sym(k), x.as_f64().unwrap_or(0.0))).collect()).unwrap_or_default()
}

fn total_yield(row: &Value, y: &str) -> f64 {
    row["cityYields"].as_array().map(|a| a.iter().map(|c| c["modified"][y].as_f64().unwrap_or(0.0)).sum()).unwrap_or(0.0)
}

impl Record {
    /// `gifts`: units the scenario gives on a turn (tribal villages), not bought
    pub fn load(rows: &Value, gifts: &HashMap<i64, Vec<Sym>>) -> Record {
        let rows = rows.as_array().unwrap();
        let mut turns = BTreeMap::new();
        for d in rows {
            let t = d["turn"].as_i64().unwrap();
            let p = &d["player"];
            let ids: HashMap<i64, Sym> = d["cities"].as_array().map(|a| a.iter().map(|c| (c["id"].as_i64().unwrap_or(0), sym(c["name"].as_str().unwrap_or(""))))
                .collect()).unwrap_or_default();
            let ys = d["cityYields"].as_array().cloned().unwrap_or_default();
            let cities = d["cities"].as_array().map(|a| a.iter().enumerate().map(|(ci, c)| RecCity {
                food_yield: ys.get(ci).map(|y| y["modified"]["YIELD_FOOD"].as_f64().unwrap_or(0.0)).unwrap_or(0.0),
                prod_yield: ys.get(ci).map(|y| y["modified"]["YIELD_PRODUCTION"].as_f64().unwrap_or(0.0)).unwrap_or(0.0),
                name: sym(c["name"].as_str().unwrap_or("")),
                center: xy_of(&c["center"]),
                pop: c["population"].as_i64().unwrap_or(0),
                food: c["foodStore"].as_f64().unwrap_or(0.0),
                current: osym(&c["production"]["current"]),
                progress: osym(&c["production"]["current"]).and_then(|k| c["production"]["progress"][k.as_string().as_str()].as_f64()).unwrap_or(0.0),
                worked: c["worked"].as_array().map(|w| w.iter().map(xy_of).collect()).unwrap_or_default(),
                owned: c["owned"].as_array().map(|w| w.iter().map(xy_of).collect()).unwrap_or_default(),
                buildings: syms(&c["buildings"]),
                districts: c["districts"].as_array().map(|a| a.iter().filter_map(|x| Some((osym(&x["type"])?, xy_of(x), x["complete"].as_bool().unwrap_or(true))))
                    .collect()).unwrap_or_default(),
            }).collect()).unwrap_or_default();
            turns.insert(t, RecTurn {
                techs: syms(&p["techs"]),
                civics: syms(&p["civics"]),
                tech_progress: fmap(&p["techProgress"]),
                civic_progress: fmap(&p["civicProgress"]),
                science: total_yield(d, "YIELD_SCIENCE"),
                culture: total_yield(d, "YIELD_CULTURE"),
                food_prod: total_yield(d, "YIELD_FOOD") + total_yield(d, "YIELD_PRODUCTION"),
                player_food_prod: p["yields"]["YIELD_FOOD"].as_f64().unwrap_or(0.0) + p["yields"]["YIELD_PRODUCTION"].as_f64().unwrap_or(0.0),
                gov: osym(&p["government"]),
                policies: syms(&p["policies"]),
                governors: p["governors"].as_array().map(|a| a.iter().filter_map(|g| Some((osym(&g["type"])?,
                    g["cityId"].as_i64().and_then(|i| ids.get(&i).copied()), syms(&g["promotions"])))).collect()).unwrap_or_default(),
                pantheon: osym(&p["pantheon"]),
                gold: p["gold"].as_f64().unwrap_or(0.0),
                gold_income: p["goldPerTurn"].as_f64().unwrap_or_else(|| total_yield(d, "YIELD_GOLD")),
                faith_income: total_yield(d, "YIELD_FAITH"),
                faith: p["faithStock"].as_f64().or(p["faith"].as_f64()).unwrap_or(0.0),
                units_trained: p["unitsTrained"].as_object().map(|o| o.iter().map(|(k, v)| (sym(k), v.as_i64().unwrap_or(0))).collect()).unwrap_or_default(),
                dedications: syms(&p["dedications"]),
                boosted: syms(&p["techsBoosted"]).into_iter().chain(syms(&p["civicsInspired"])).collect(),
                great_people: p["greatPeopleRecruited"].as_array().map(|a| a.iter().filter_map(|g| osym(&g["individual"])).collect()).unwrap_or_default(),
                cities,
            });
        }
        let mut rec = Record { turns, research: HashMap::new(), civic: HashMap::new(), acts: vec![], purchases: vec![], routes: vec![], sites: vec![],
                               tiles: HashMap::new(), first_owned: HashMap::new() };
        rec.read_choices(rows, gifts);
        rec
    }

    fn read_choices(&mut self, rows: &[Value], gifts: &HashMap<i64, Vec<Sym>>) {
        let keys: Vec<i64> = self.turns.keys().copied().collect();
        let mut seen_imp: HashSet<(XY, Sym)> = HashSet::new();
        let mut seen_city: HashSet<Sym> = HashSet::new();
        let mut seen_route: HashSet<(i64, Sym, Sym)> = HashSet::new();
        for (i, &t) in keys.iter().enumerate() {
            let a = &self.turns[&t];
            for c in &a.cities {
                if seen_city.insert(c.name) {
                    self.first_owned.insert(c.center, c.owned.clone());
                    if i > 0 {
                        self.sites.push((t - 1, c.name, c.center));
                    }
                }
            }
            if let Some(d) = rows.iter().find(|d| d["turn"].as_i64() == Some(t)) {
                for r in d["tradeRoutes"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
                    let (Some(o), Some(de)) = (osym(&r["origin"]["name"]), osym(&r["destination"]["name"])) else { continue };
                    let k = (r["started"].as_i64().unwrap_or(t), o, de);
                    if seen_route.insert(k) {
                        self.routes.push(k);
                    }
                }
            }
            let Some(&tn) = keys.get(i + 1) else { continue };
            let b = &self.turns[&tn];
            // research / civic of turn t
            // (one boosted this turn grew by its boost, not by research, unless nothing else grew)
            let boosted_now: Vec<Sym> = b.boosted.iter().filter(|x| !a.boosted.contains(x)).copied().collect();
            let pick = |done_a: &Vec<Sym>, done_b: &Vec<Sym>, pa: &HashMap<Sym, f64>, pb: &HashMap<Sym, f64>, rate: f64| -> Option<Sym> {
                if let Some(x) = done_b.iter().find(|x| !done_a.contains(x)) {
                    return Some(*x);
                }
                let grew = |k: &Sym, v: &f64| *v > pa.get(k).copied().unwrap_or(0.0) + 1e-9;
                // in a fixed order (a map's order varies from run to run; ties must not)
                let mut pbs: Vec<(&Sym, &f64)> = pb.iter().collect();
                pbs.sort_by_key(|x| x.0.as_string());
                let unboosted = pbs.iter().any(|(k, v)| grew(k, v) && !boosted_now.contains(k));
                pbs.into_iter().filter(|(k, v)| grew(k, v) && (!unboosted || !boosted_now.contains(k)))
                    .min_by(|x, y| ((x.1 - pa.get(x.0).copied().unwrap_or(0.0)) - rate).abs().partial_cmp(&((y.1 - pa.get(y.0).copied().unwrap_or(0.0)) - rate).abs()).unwrap())
                    .map(|(k, _)| *k)
            };
            if let Some(x) = pick(&a.techs, &b.techs, &a.tech_progress, &b.tech_progress, b.science) {
                self.research.insert(t, x);
            }
            if let Some(x) = pick(&a.civics, &b.civics, &a.civic_progress, &b.civic_progress, b.culture) {
                self.civic.insert(t, x);
            }
            // builder actions, tiles. A plot's last state: any of the player's cities (a tile moved between cities and
            // then chopped is a harvest too)
            let all_a: HashMap<XY, Value> = rows.iter().find(|d| d["turn"].as_i64() == Some(t))
                .map(|d| d["cities"].as_array().unwrap().iter().flat_map(|c| c["owned"].as_array().unwrap().iter().map(|p| (xy_of(p), p.clone()))).collect())
                .unwrap_or_default();
            for cb in &b.cities {
                let ca = a.city(cb.name);
                let plots_b = rows.iter().find(|d| d["turn"].as_i64() == Some(tn)).and_then(|d| d["cities"].as_array().and_then(|cs| cs.iter().find(|c| osym(&c["name"]) == Some(cb.name)).cloned()));
                let plots_a = rows.iter().find(|d| d["turn"].as_i64() == Some(t)).and_then(|d| d["cities"].as_array().and_then(|cs| cs.iter().find(|c| osym(&c["name"]) == Some(cb.name)).cloned()));
                if let (Some(ca), Some(pa), Some(pb)) = (ca, plots_a, plots_b) {
                    let new: Vec<XY> = cb.owned.iter().filter(|xy| !ca.owned.contains(xy)).copied().collect();
                    self.tiles.entry(cb.center).or_default().extend(new.into_iter().map(|xy| (t, xy)));
                    let _ = pa;
                    let pa = &all_a;
                    for p in pb["owned"].as_array().unwrap() {
                        let xy = xy_of(p);
                        if let Some(imp) = osym(&p["improvement"]) {
                            if seen_imp.insert((xy, imp)) {
                                self.acts.push((t, s!("improve"), xy, imp));
                            }
                        }
                        let Some(q) = pa.get(&xy) else { continue };
                        if !p["district"].is_null() || !p["wonder"].is_null() {
                            continue;
                        }
                        if !q["feature"].is_null() && p["feature"].is_null() {
                            self.acts.push((t, s!("harvest"), xy, s!("feature")));
                        }
                        if !q["resource"].is_null() && p["resource"].is_null() {
                            self.acts.push((t, s!("harvest"), xy, s!("resource")));
                        }
                    }
                }
            }
            // purchases: units beyond what the cities finished; buildings that were not the city's item. Only on a
            // turn gold or faith went down by more than the turn's income explains (a purchase costs at least 15: an
            // early trader at online speed is about 20): units and buildings also come free (governors, government
            // buildings, eras)
            let spent = (a.gold + a.gold_income - b.gold).max(a.faith + a.faith_income - b.faith);
            if spent < 15.0 {
                continue;
            }
            let mut ut: Vec<(&Sym, &i64)> = b.units_trained.iter().collect();
            ut.sort_by_key(|x| x.0.as_string());
            for (u, n) in ut {
                let before = a.units_trained.get(u).copied().unwrap_or(0);
                let finished = a.cities.iter().filter(|ca| ca.current == Some(*u) && b.city(ca.name).map(|cb| cb.current != Some(*u)).unwrap_or(false)).count() as i64;
                let given = gifts.get(&t).map(|g| g.iter().filter(|x| *x == u).count() as i64).unwrap_or(0);
                for _ in 0..(n - before - finished - given).max(0) {
                    let city = a.cities.first().map(|c| c.name).unwrap_or(Sym(0));
                    self.purchases.push((t, city, *u));
                }
            }
            // (bought buildings: the replay takes buildings done in the next save as done, Planner::advance)
        }
        // the first recorded turn's improvements were there from the start
        let t0 = keys.first().copied().unwrap_or(0);
        self.acts.retain(|a| a.0 > t0 || a.1 != s!("improve"));
    }

    /// The items a city worked on in order: a new entry where the item changes or its progress went down (finished,
    /// bought, or carried over).
    pub fn prod_sequence(&self, city: Sym) -> Vec<Sym> {
        let mut out: Vec<Sym> = vec![];
        let mut last: Option<(Sym, f64)> = None;
        for r in self.turns.values() {
            let Some(c) = r.city(city) else { continue };
            let Some(cur) = c.current else { continue };
            let new = match last { Some((i, p)) => i != cur || c.progress < p, None => true };
            if new {
                out.push(cur);
            }
            last = Some((cur, c.progress));
        }
        out
    }

    /// The techs (civics) in the order the record completed them.
    pub fn completion_order(&self, civic: bool) -> Vec<Sym> {
        let mut out: Vec<Sym> = vec![];
        for r in self.turns.values() {
            for x in if civic { &r.civics } else { &r.techs } {
                if !out.contains(x) {
                    out.push(*x);
                }
            }
        }
        out
    }

    pub fn turn(&self, t: i64) -> Option<&RecTurn> {
        self.turns.get(&t)
    }

    /// Gold the player gave away on each turn: what the gold went down by beyond the turn's income on a turn with no
    /// purchase (no unit or building bought, no tile bought) to explain it. `gifts_in`: gold that came in on a turn
    /// (goody huts, gifts from others).
    pub fn gold_given(&self, gifts_in: &HashMap<i64, f64>) -> HashMap<i64, f64> {
        let mut out = HashMap::new();
        let mut turns: Vec<i64> = self.turns.keys().copied().collect();
        turns.sort();
        for t in turns {
            let (Some(a), Some(b)) = (self.turn(t), self.turn(t + 1)) else { continue };
            let drop = a.gold + a.gold_income + gifts_in.get(&t).copied().unwrap_or(0.0) - b.gold;
            if drop < 0.5 {
                continue;
            }
            let bought_item = self.purchases.iter().any(|p| p.0 == t);
            let bought_tile = drop >= 20.0 && self.tiles.values().any(|v| v.iter().any(|(tt, _)| *tt == t));
            if !bought_item && !bought_tile {
                out.insert(t, drop);
            }
        }
        out
    }

    /// The recorded district of `dist` (or one replacing it) in a city, where it stands.
    pub fn district_at(&self, city: Sym, dist: Sym) -> Option<XY> {
        self.turns.values().rev().find_map(|t| t.city(city).and_then(|c| c.districts.iter().find(|d| d.0 == dist).map(|d| d.1)))
    }
}

/// Chooses as the recorded game did; where the record has nothing usable, the planner's base policy.
pub struct RecordedChooser<'r> {
    pub rec: &'r Record,
    done_acts: RefCell<HashSet<usize>>,
    done_purchases: HashSet<usize>,
    done_routes: HashSet<usize>,
    /// under the model's rules: per city, how far along its recorded item sequence the choices are
    prod_at: HashMap<Sym, usize>,
}

impl<'r> RecordedChooser<'r> {
    pub fn new(rec: &'r Record) -> Self {
        RecordedChooser { rec, done_acts: Default::default(), done_purchases: Default::default(), done_routes: Default::default(), prod_at: Default::default() }
    }
}

impl Chooser for RecordedChooser<'_> {
    fn choose(&mut self, p: &Planner, st: &State, key: &Key, options: &[Choice]) -> Option<Choice> {
        let rec = self.rec;
        let t = st.turn;
        let k = key.kind;
        let has = |c: &Choice| options.contains(c);
        let next = rec.turn(t + 1).or_else(|| rec.turn(t));
        if (k == s!("research") || k == s!("civic")) && p.rec.is_none() {
            // under the model's rules a choice holds until it completes (no switching): the next one the record
            // completed that is available now
            if let Some(x) = rec.completion_order(k == s!("civic")).into_iter().map(Choice::S).find(has) {
                return Some(x);
            }
        }
        if k == s!("research") || k == s!("civic") {
            let m = if k == s!("research") { &rec.research } else { &rec.civic };
            // this turn's, else the next one the record works on that is available now
            if let Some(x) = m.get(&t).map(|x| Choice::S(*x)).filter(has) {
                return Some(x);
            }
            let mut later: Vec<(&i64, &Sym)> = m.iter().filter(|(tt, _)| **tt > t).collect();
            later.sort();
            if let Some((_, x)) = later.into_iter().find(|(_, x)| has(&Choice::S(**x))) {
                return Some(Choice::S(*x));
            }
        } else if k == s!("gov") {
            if let Some(g) = next.and_then(|r| r.gov).map(Choice::S).filter(has) {
                return Some(g);
            }
            return st.player.government.map(Choice::S).filter(has).or_else(|| p.base(st, key, options));
        } else if k == s!("policy") {
            if let Some(r) = next {
                if let Some(x) = r.policies.iter().map(|x| Choice::S(*x)).find(has) {
                    return Some(x);
                }
            }
            return Some(Choice::S(s!("NONE")));
        } else if k == s!("pantheon") {
            let last = rec.turns.values().rev().find_map(|r| r.pantheon);
            if let Some(x) = last.map(Choice::S).filter(has) {
                return Some(x);
            }
        } else if k == s!("governor") {
            if let Some(r) = next {
                for (g, city, promos) in &r.governors {
                    let mine = st.player.governors.iter().flatten().find(|x| x.gtype == *g);
                    match mine {
                        None => {
                            if let Some(c) = city {
                                if has(&Choice::Appoint(*g, *c)) {
                                    return Some(Choice::Appoint(*g, *c));
                                }
                            }
                            if let Some(o) = options.iter().find(|o| matches!(o, Choice::Appoint(gg, _) if gg == g)) {
                                return Some(o.clone());
                            }
                        }
                        Some(m) => {
                            for pr in promos {
                                if !m.promotions.contains(pr) && has(&Choice::Promote(*g, *pr)) {
                                    return Some(Choice::Promote(*g, *pr));
                                }
                            }
                        }
                    }
                }
            }
            return Some(Choice::Nothing).filter(has).or_else(|| p.base(st, key, options));
        } else if k == s!("prod") {
            if crate::envcfg::trace() == Some("prod") {
                eprintln!("PROD OPTIONS T{t} {:?}: {:?}", key.ent, options.iter().filter_map(|o| o.sym()).map(|x| x.as_string()).collect::<Vec<_>>());
            }
            if let (Ent::S(city), true) = (&key.ent, p.rec.is_none() && p.tiles_rec.is_some()) {
                // the recorded player's play replayed (items switched as recorded): the item the record works on now
                if let Some(x) = rec.turn(t).and_then(|r| r.city(*city)).and_then(|c| c.current).map(Choice::S).filter(has) {
                    return Some(x);
                }
            }
            if let (Ent::S(city), true) = (&key.ent, p.rec.is_none()) {
                // under the model's rules the timing differs: the next item of the city's recorded sequence that is
                // available now
                let seq = rec.prod_sequence(*city);
                let at = self.prod_at.get(city).copied().unwrap_or(0);
                if let Some(j) = (at..seq.len()).find(|&j| has(&Choice::S(seq[j]))) {
                    self.prod_at.insert(*city, j + 1);
                    return Some(Choice::S(seq[j]));
                }
            }
            if let Ent::S(city) = key.ent {
                for tt in [t, t + 1] {
                    if let Some(x) = rec.turn(tt).and_then(|r| r.city(city)).and_then(|c| c.current).map(Choice::S).filter(has) {
                        return Some(x);
                    }
                }
            }
        } else if k == s!("builder") {
            let mut done = self.done_acts.borrow_mut();
            for (i, a) in rec.acts.iter().enumerate() {
                if a.0 > t || done.contains(&i) {
                    continue;
                }
                if let Some(o) = options.iter().find(|o| matches!(o, Choice::Act(kk, _, x, y, w) if *kk == a.1 && (*x, *y) == a.2 && *w == a.3)) {
                    done.insert(i);
                    return Some(o.clone());
                }
            }
            return Some(Choice::Wait).filter(has).or_else(|| options.last().cloned());
        } else if k == s!("buy") {
            // up to two turns late: an item unlocked during the recorded turn (an inspiration's civic) is the model's
            // at the next turn's start
            for (i, (pt, city, item)) in rec.purchases.iter().enumerate() {
                if *pt > t || *pt + 2 < t || self.done_purchases.contains(&i) {
                    continue;
                }
                let found = options.iter().find(|o| matches!(o, Choice::BuyItem(c, it, _, cur) if c == city && it == item && *cur == s!("YIELD_GOLD")))
                    .or_else(|| options.iter().find(|o| matches!(o, Choice::BuyItem(c, it, _, _) if c == city && it == item)))
                    .or_else(|| options.iter().find(|o| matches!(o, Choice::BuyItem(_, it, _, _) if it == item)));
                if let Some(o) = found {
                    self.done_purchases.insert(i);
                    return Some(o.clone());
                }
            }
            // under the model's rules (the replay takes the recorded tiles as they are): a plot the record's city got
            // on a turn its gold went down by more than the income explains was bought
            if p.rec.is_none() {
                if let (Some(a), Some(b)) = (rec.turn(t), rec.turn(t + 1)) {
                    if a.gold + a.gold_income - b.gold >= 20.0 {
                        let got = |xy: XY| rec.tiles.values().any(|v| v.iter().any(|(tt, x)| *tt == t && *x == xy));
                        if let Some(o) = options.iter().find(|o| matches!(o, Choice::BuyTile(_, _, x, y) if got((*x, *y)))) {
                            return Some(o.clone());
                        }
                    }
                }
            }
            return Some(Choice::Nothing);
        } else if k == s!("settle") {
            for (_, _, xy) in &rec.sites {
                let c = Choice::Xy(xy.0, xy.1);
                if has(&c) && !st.cities.iter().any(|c| c.center == *xy) {
                    return Some(c);
                }
            }
        } else if k == s!("trade") {
            // the recorded route nearest in time that is on offer: its home and destination, else its destination
            let mut best: Option<(i64, usize, Choice)> = None;
            for (i, (rt, o, d)) in rec.routes.iter().enumerate() {
                if self.done_routes.contains(&i) {
                    continue;
                }
                let c = Some(Choice::Route(*o, *d)).filter(has)
                    .or_else(|| options.iter().find(|x| matches!(x, Choice::Route(_, dd) if dd == d)).cloned());
                let Some(c) = c else { continue };
                // an exact match (home and destination) before one with the destination only
                let gap = (rt - t).abs() + if c == Choice::Route(*o, *d) { 0 } else { 1000 };
                if best.as_ref().map(|b| gap < b.0).unwrap_or(true) {
                    best = Some((gap, i, c));
                }
            }
            if let Some((_, i, c)) = best {
                self.done_routes.insert(i);
                return Some(c);
            }
        } else if k == s!("gp") {
            if let Some(r) = next {
                if let Some(x) = options.iter().find(|o| o.sym().map(|s| r.great_people.contains(&s)).unwrap_or(false)) {
                    return Some(x.clone());
                }
            }
            return Some(Choice::S(s!("wait"))).filter(has).or_else(|| p.base(st, key, options));
        } else if k == s!("dedication") {
            for r in [rec.turn(t), next].into_iter().flatten() {
                if let Some(x) = r.dedications.iter().map(|x| Choice::S(*x)).find(|c| has(c) && !st.player.dedications.contains(&c.sym().unwrap())) {
                    return Some(x);
                }
            }
        } else if k == s!("focus") {
            return Some(Choice::S(s!("balanced"))).filter(has).or_else(|| p.base(st, key, options));
        }
        p.base(st, key, options)
    }
}
