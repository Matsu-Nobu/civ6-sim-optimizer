//! One-turn economy step (civ6sim/sim.py).
use crate::amenities::{amenity_scores, happiness};
use crate::engine::{ss, Engine, EngineCache};
use crate::growth::{food_added, growth_threshold};
use crate::model::*;
use crate::production::*;
use crate::rules::{Rules, Y};
use crate::sym::Sym;
use crate::yields::{city_yield_percents, city_yields, faith_conversions, get_y, player_yields, tile_yields};

pub fn research_cost(r: &Rules, item: Sym, speed: Sym, game_era: Sym) -> f64 {
    let is_tech = item.starts_with("TECH_");
    let (table, col) = if is_tech { ("Technologies", "TechnologyType") } else { ("Civics", "CivicType") };
    let base = r.lookup_f(table, col, item, "Cost");
    let era = r.lookup_sym(table, col, item, "EraType").unwrap();
    let (oe, og) = (r.era_index(era).unwrap_or(0), r.era_index(game_era).unwrap_or(0));
    let pct = if oe > og {
        r.param(if is_tech { "TECH_COST_PERCENT_CHANGE_AFTER_GAME_ERA" } else { "CIVIC_COST_PERCENT_CHANGE_AFTER_GAME_ERA" })
    } else if oe < og {
        r.param(if is_tech { "TECH_COST_PERCENT_CHANGE_BEFORE_GAME_ERA" } else { "CIVIC_COST_PERCENT_CHANGE_BEFORE_GAME_ERA" })
    } else {
        0.0
    };
    (base.floor() * r.game_speed_cost_multiplier(speed) * (1.0 + pct / 100.0) + 1e-9).floor()
}

pub fn boost_amount(cost: f64, extra_pct: f64) -> f64 {
    ((0.4 + extra_pct / 100.0) * (cost - 2.5)).floor()
}

/// The city's yields this turn with population `pop`, after % modifiers.
pub fn city_turn_yields(r: &Rules, e: &Engine, city: &City, pop: i32, routes: &[Route], amenity_pct: f64, amenity_score: Option<f64>) -> Y {
    let mut c = city.clone();
    c.population = pop;
    let y = city_yields(r, e, &c, routes);
    let extra = city_yield_percents(r, e, &c, amenity_score);
    let mut out = [0.0; 6];
    for i in 0..6 {
        let am = if i == 0 { 0.0 } else { amenity_pct };
        out[i] = y[i] * (1.0 + (am + extra[i]) / 100.0);
    }
    for (k, v) in faith_conversions(e, &c, &out) {
        crate::yields::add(&mut out, k, v);
    }
    out
}

fn completion_yields(r: &Rules, e: &Engine, c: &City, item: Sym, cost: f64) -> Vec<(Sym, f64)> {
    let mut out = vec![];
    match item_kind(item) {
        Kind::Building => {
            let wonder = r.is_wonder(item);
            for x in e.city_effects(s!("EFFECT_GRANT_CITY_YIELD_PERCENT_BUILDING_CREATED_COST"), c.name) {
                if wonder && !x.args.is("IncludeWonder", &["1", "true"]) {
                    continue;
                }
                crate::yields::push_y(&mut out, x.args.get("YieldType").unwrap(), cost * x.args.f("BuildingProductionPercent") / 100.0);
            }
        }
        Kind::Unit => {
            for x in e.city_effects(s!("EFFECT_GRANT_CITY_YIELD_PERCENT_UNIT_CREATED_COST"), c.name) {
                crate::yields::push_y(&mut out, x.args.get("YieldType").unwrap(), cost * x.args.f("UnitProductionPercent") / 100.0);
            }
        }
        _ => {}
    }
    out
}

#[derive(Clone, Debug)]
pub enum Event {
    Completed(Sym, Sym),
    CompletedPurchase(Sym, Sym),
    Grew(Sym, i32),
    Researched(Sym),
    /// the yields a city's growth and production used this turn
    Yields(Sym, Y),
}

const PCT_TOTALS: [usize; 4] = [2, 3, 4, 5]; // gold, science, culture, faith

/// Advance one turn (sim.step). `cities` are the state's cities (populations already set to the simulator's).
#[allow(clippy::too_many_arguments)]
/// Districts placed per type and the specialty districts completed (item costs depend on both).
fn placed_districts(r: &Rules, cities: &[City]) -> (Vec<(Sym, i64)>, i64) {
    let mut placed: Vec<(Sym, i64)> = vec![];
    let mut completed = 0i64;
    for c in cities {
        for d in &c.districts {
            if d.dtype == s!("DISTRICT_CITY_CENTER") {
                continue;
            }
            if let Some(x) = placed.iter_mut().find(|(k, _)| *k == d.dtype) { x.1 += 1 } else { placed.push((d.dtype, 1)) }
            if d.complete && r.district_requires_population(d.dtype) {
                completed += 1;
            }
        }
    }
    (placed, completed)
}

/// What the city's item costs now, as step charges it (a district at the cost locked when it was begun).
pub fn current_cost(r: &Rules, acc: &PlayerAcc, cities: &[City], env: &Env, city: Sym, item: Sym, speed: Sym) -> Option<f64> {
    if let Some(v) = acc.cities.get(&city).and_then(|ca| ca.locked_cost.get(&item)) {
        return Some(*v);
    }
    let (placed, completed) = placed_districts(r, cities);
    let mut env_cost = env.clone();
    env_cost.techs = acc.techs.clone();
    env_cost.civics = acc.civics.clone();
    env_cost.units_trained = acc.units_trained.clone();
    env_cost.districts_completed = Some(acc.districts_snapshot.unwrap_or(completed));
    item_cost(r, &env_cost, item, speed, Some(&placed), false)
}

/// `after_settler`: a city's worked tiles once a settler has taken one of its citizens (the game reassigns them all),
/// given the city and its engine.
#[allow(clippy::too_many_arguments)]
pub fn step<'a>(r: &'a Rules, ec: &mut EngineCache<'a>, acc: &mut PlayerAcc, cities: &[City], env: &Env, units: &OMap<Sym, i64>, speed: Sym,
                research: Option<Sym>, civic: Option<Sym>, boosts: &[Sym], routes: &[Route],
                after_settler: &dyn Fn(&City, &Engine) -> City) -> Vec<Event> {
    let mut log = vec![];
    for c in cities {
        if !acc.cities.contains(&c.name) {
            acc.cities.insert(c.name, CityAcc { population: c.population, ..Default::default() });
        }
    }
    let sim_cities: Vec<City> = cities.iter().map(|c| { let mut x = c.clone(); x.population = acc.cities.get(&c.name).unwrap().population; x }).collect();
    let (placed, completed) = placed_districts(r, cities);
    let e_sim = ec.get(r, env, &sim_cities);
    let scores = amenity_scores(r, &sim_cities, &e_sim, env.extra_luxuries);
    // Civ6's end of turn (CivFanatics, "When does production finish?"): techs and civics complete nation-wide first
    // (from the science and culture of the turn), then each city's growth and production, which see them
    let env_new;
    let env = {
        let e0 = ec.get(r, env, cities);
        let mut pre: Y = [0.0; 6];
        for c in cities {
            let score = scores.iter().find(|(n, _)| *n == c.name).map(|x| x.1).unwrap_or(0.0);
            let pop = acc.cities.get(&c.name).unwrap().population;
            let y = city_turn_yields(r, &e0, c, pop, routes, happiness(r, score).0, Some(score));
            for &k in &PCT_TOTALS {
                pre[k] += y[k];
            }
        }
        let py = player_yields(r, &e0, &pre);
        for &k in &PCT_TOTALS {
            pre[k] += py[k];
        }
        research_step(r, &e0, acc, &pre, speed, research, civic, boosts, env.game_era, completed, &mut log);
        let mut x = env.clone();
        x.techs = acc.techs.clone();
        x.civics = acc.civics.clone();
        env_new = x;
        &env_new
    };
    let e = ec.get(r, env, cities);
    let mut totals: Y = [0.0; 6];
    let mut pending: Vec<(&City, Y, Option<City>, bool, f64, f64, f64)> = vec![];
    for c in cities {
        let score = scores.iter().find(|(n, _)| *n == c.name).map(|x| x.1).unwrap_or(0.0);
        let (nonfood_pct, growth_pct) = happiness(r, score);
        let pop = acc.cities.get(&c.name).unwrap().population;
        let y = city_turn_yields(r, &e, c, pop, routes, nonfood_pct, Some(score));
        if crate::envcfg::trace_city(&c.name.as_string()) {
            let mods: &[crate::engine::PlotMod] = e.plot_effects.get(&c.name).map(|v| v.as_slice()).unwrap_or(&[]);
            let tiles: Vec<_> = c.worked.iter().map(|p| (p.xy(), crate::yields::to_y(&crate::yields::tile_yields(r, p, &env.techs, &env.civics, p.xy() == c.center, mods, Some(&e.plots), Some(c), None)))).collect();
            eprintln!("YIELDS {} pop {pop} {:?} tiles {:?}", c.name, y, tiles);
        }
        // the game shows a flat bonus to the item being built (e.g. +1 toward units) as the city's production
        let mut shown = y;
        if let Some(item) = c.current {
            shown[1] += item_production_flat(&e, c, item);
        }
        log.push(Event::Yields(c.name, shown));
        let mut y = y;
        for &k in &PCT_TOTALS {
            totals[k] += y[k];
        }
        let item = c.current;
        if let Some(item) = item {
            if crate::envcfg::trace_city(&c.name.as_string()) {
                let mut env_cost = env.clone();
                env_cost.units_trained = acc.units_trained.clone();
                let cost = item_cost(r, &env_cost, item, speed, Some(&placed), false);
                let ca = acc.cities.get(&c.name).unwrap();
                eprintln!("PROD {} {item} P {} pct {} flat {} progress {:?} overflow {} cost {:?}", c.name, y[1], item_production_percent(r, &e, c, item), item_production_flat(&e, c, item), ca.progress.get(&item), ca.overflow, cost);
            }
        }
        let mut settled = false;
        let mut food_city: Option<City> = None;
        let ca = acc.cities.get_mut(&c.name).unwrap();
        // a city working on nothing keeps its production for the next item, whatever its progress (10/17 Susa T20-21)
        if item.is_none() {
            ca.raw += y[1];
        }
        if let Some(item) = item {
            let pct = item_production_percent(r, &e, c, item);
            let flat = item_production_flat(&e, c, item);
            // a chop finished it during the turn: the turn's production goes to the next item (CivFanatics, "Is
            // chopping overflow overcorrected?"), here a turn later
            // the carried-over production goes into the item with the item's percent bonus (10/18 Nantes T29: (4.5 + 9)
            // x 1.5; T18: 5 + 8 + 1)
            let before = ca.progress.get(&item).copied().unwrap_or(0.0);
            let applied = ca.overflow;
            let own = y[1] + flat;
            let base = if ca.chopped == Some(item) {
                ca.raw += y[1];
                0.0
            } else {
                let x = own + ca.raw;
                ca.raw = 0.0;
                x
            };
            ca.chopped = None;
            let into = base * (1.0 + pct / 100.0);
            *ca.progress.entry_or(item, 0.0) += (base + applied) * (1.0 + pct / 100.0);
            ca.overflow -= applied;
            if item_kind(item) == Kind::Project {
                for (k, pc) in r.yield_rows("Project_YieldConversions", "ProjectType", item, "YieldType", "PercentOfProductionRate") {
                    if k == s!("YIELD_FOOD") {
                        y[0] += into * pc / 100.0;
                    } else if let Some(i) = crate::rules::yidx(k) {
                        if PCT_TOTALS.contains(&i) {
                            totals[i] += into * pc / 100.0;
                        }
                    }
                }
            }
            let mut env_cost = env.clone();
            env_cost.techs = acc.techs.clone();
            env_cost.civics = acc.civics.clone();
            env_cost.units_trained = acc.units_trained.clone();
            env_cost.districts_completed = Some(acc.districts_snapshot.unwrap_or(completed));
            let ca = acc.cities.get_mut(&c.name).unwrap();
            let cost = if item_kind(item) == Kind::District {
                if !ca.locked_cost.contains(&item) {
                    let v = item_cost(r, &env_cost, item, speed, Some(&placed), false).unwrap();
                    ca.locked_cost.insert(item, v);
                }
                Some(*ca.locked_cost.get(&item).unwrap())
            } else {
                item_cost(r, &env_cost, item, speed, Some(&placed), false)
            };
            if let Some(cost) = cost {
                let prog = *ca.progress.get(&item).unwrap();
                if prog >= cost {
                    // the overflow is what the item's progress and this turn's production without its percent bonus
                    // have beyond the cost: none when it needed the bonus to finish (Civ6 wiki, Production (Civ6),
                    // Gathering Storm), and the carried-over production put into it does not carry on (the saves:
                    // 10/19 Cumana T22, 10/14 Paris T29, 10/18 La Rochelle T22)
                    ca.overflow = if before + base + applied < cost { 0.0 } else { (before + own.min(base) - cost).max(0.0) };
                    ca.progress.insert(item, 0.0);
                    ca.locked_cost.remove(&item);
                    log.push(Event::Completed(c.name, item));
                    for (k, v) in completion_yields(r, &e, c, item, cost) {
                        if let Some(i) = crate::rules::yidx(k) {
                            if PCT_TOTALS.contains(&i) {
                                totals[i] += v;
                            }
                        }
                    }
                    // the city grows with what it just finished (a building, or a district: e.g. BBG Autocracy's monument
                    // food for a city with a district)
                    if item_kind(item) == Kind::Building {
                        let mut fc = c.clone();
                        fc.buildings.push(item);
                        food_city = Some(fc);
                    } else if item_kind(item) == Kind::District {
                        let mut fc = c.clone();
                        for d in fc.districts.iter_mut() {
                            if d.dtype == item {
                                d.complete = true;
                            }
                        }
                        food_city = Some(fc);
                    }
                    if item_kind(item) == Kind::Unit {
                        *acc.units_trained.entry_or(item, 0) += 1;
                        let mut pop_cost = r.lookup_f("Units", "UnitType", item, "PopulationCost") as i32;
                        if e.city_effects(s!("EFFECT_ADJUST_CITY_SETTLER_CONSUME_POP"), c.name).iter().any(|x| x.args.is("Enabled", &["0", "false"])) {
                            pop_cost = 0;
                        }
                        if pop_cost != 0 {
                            acc.cities.get_mut(&c.name).unwrap().population -= pop_cost;
                            settled = true;
                        }
                    }
                }
            }
        }
        pending.push((c, y, food_city, settled, score, nonfood_pct, growth_pct));
    }
    // the cities grow after all have produced: a trade route's food counts the districts its destination finished
    // this turn (the records: 10/14 La Rochelle's route to Paris, whose campus was finished at T24)
    let mut routes_now: Vec<Route> = routes.to_vec();
    for ev in &log {
        if let Event::Completed(n, item) = ev {
            if item_kind(*item) == Kind::District {
                for rt in routes_now.iter_mut().filter(|rt| rt.dest.name == *n && rt.dest.player_id == rt.origin.player_id) {
                    if !rt.dest.districts.contains(item) {
                        rt.dest.districts.push(*item);
                    }
                }
            }
        }
    }
    for (c, y, food_city, settled, score, nonfood_pct, growth_pct) in pending {
        let routes_changed = routes_now.iter().zip(routes.iter()).any(|(a, b)| a.origin.name == c.name && a.dest.districts != b.dest.districts);
        let routes = &routes_now[..];
        let pop = acc.cities.get(&c.name).unwrap().population;
        let mut food = y[0];
        if routes_changed && food_city.is_none() {
            food = city_turn_yields(r, &e, c, pop, routes, nonfood_pct, Some(score))[0];
        }
        if let Some(fc) = &food_city {
            let now: Vec<City> = cities.iter().map(|x| if x.name == fc.name { fc.clone() } else { x.clone() }).collect();
            let ef = ec.get(r, env, &now);
            food = city_turn_yields(r, &ef, fc, pop, routes, nonfood_pct, Some(score))[0];
        }
        let mut growth_pct = growth_pct;
        if settled {
            // the city's amenities for growth count the citizens left after the settler
            let now: Vec<City> = sim_cities.iter().map(|x| { let mut x = x.clone(); if x.name == c.name { x.population = pop; } x }).collect();
            let score_now = amenity_scores(r, &now, &e_sim, env.extra_luxuries).iter().find(|(n, _)| *n == c.name).map(|x| x.1).unwrap_or(0.0);
            growth_pct = happiness(r, score_now).1;
            let mut fewer = c.clone();
            fewer.population = pop;
            let fewer = after_settler(&fewer, &e);
            food = city_turn_yields(r, &e, &fewer, pop, routes, 0.0, None)[0];
        }
        let fc = food_city.as_ref().unwrap_or(c);
        let add = food_added(r, fc, pop, food, growth_pct, Some(&e));
        if crate::envcfg::trace_city(&c.name.as_string()) {
            eprintln!("FOOD {} pop {pop} food {food} growth_pct {growth_pct} add {add} housing {}", c.name, crate::growth::housing(r, fc, Some(&e)));
        }
        let ca = acc.cities.get_mut(&c.name).unwrap();
        ca.food_store += add;
        let need = growth_threshold(r, ca.population, speed);
        if ca.food_store >= need {
            ca.food_store -= need;
            ca.population += 1;
            log.push(Event::Grew(c.name, ca.population));
        }
    }
    let py = player_yields(r, &e, &totals);
    for &k in &PCT_TOTALS {
        totals[k] += py[k];
    }
    totals[2] -= maintenance(r, &e, cities, units);
    acc.gold += totals[2];
    acc.faith += totals[5];
    if acc.districts_snapshot.is_none() {
        acc.districts_snapshot = Some(completed);
    }
    log
}

#[allow(clippy::too_many_arguments)]
fn research_step(r: &Rules, e: &Engine, acc: &mut PlayerAcc, totals: &Y, speed: Sym, research: Option<Sym>, civic: Option<Sym>, boosts: &[Sym],
                 game_era: Sym, completed: i64, log: &mut Vec<Event>) {
    let extra_t: f64 = e.player_effects(s!("EFFECT_ADJUST_TECHNOLOGY_BOOST")).iter().map(|x| x.args.f("Amount")).sum();
    let extra_c: f64 = e.player_effects(s!("EFFECT_ADJUST_CIVIC_BOOST")).iter().map(|x| x.args.f("Amount")).sum();
    for is_tech in [true, false] {
        let amount = if is_tech { totals[3] } else { totals[4] };
        let cur = if is_tech { research } else { civic };
        let mut boosted_now: Vec<(Sym, f64)> = vec![];
        {
            let (prog, done) = if is_tech { (&mut acc.tech_progress, &acc.techs) } else { (&mut acc.civic_progress, &acc.civics) };
            for &b in boosts {
                if b.starts_with("TECH_") == is_tech && !done.contains(&b) && r.has_boost(b) {
                    let amt = boost_amount(research_cost(r, b, speed, game_era), if is_tech { extra_t } else { extra_c });
                    *prog.entry_or(b, 0.0) += amt;
                    boosted_now.push((b, amt));
                }
            }
        }
        let over_now = if is_tech { acc.tech_overflow } else { acc.civic_overflow };
        if let Some(cur) = cur {
            let done = if is_tech { &acc.techs } else { &acc.civics };
            if !done.contains(&cur) {
                let prog = if is_tech { &mut acc.tech_progress } else { &mut acc.civic_progress };
                *prog.entry_or(cur, 0.0) += amount + over_now;
                if is_tech { acc.tech_overflow = 0.0 } else { acc.civic_overflow = 0.0 }
            }
        }
        let finished: Vec<Sym> = {
            let (prog, done) = if is_tech { (&acc.tech_progress, &acc.techs) } else { (&acc.civic_progress, &acc.civics) };
            prog.iter().filter(|(t, v)| !done.contains(t) && *v >= research_cost(r, *t, speed, game_era)).map(|(t, _)| *t).collect()
        };
        for t in finished {
            let prog = if is_tech { &mut acc.tech_progress } else { &mut acc.civic_progress };
            let over = prog.remove(&t).unwrap() - research_cost(r, t, speed, game_era);
            let b = boosted_now.iter().find(|(k, _)| *k == t).map(|x| x.1).unwrap_or(0.0);
            let add = (over - b).max(0.0);
            if is_tech { acc.tech_overflow += add; acc.techs.insert(t); } else { acc.civic_overflow += add; acc.civics.insert(t); }
            log.push(Event::Researched(t));
            acc.districts_snapshot = Some(completed);
        }
    }
}
