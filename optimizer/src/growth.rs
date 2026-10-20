//! City growth (civ6sim/growth.py): housing, the food for the next citizen, how surplus food accumulates.
use crate::engine::{ss, Engine};
use crate::geo::neighbour_xy;
use crate::model::*;
use crate::rules::Rules;
use crate::sym::Sym;

pub fn growth_threshold(r: &Rules, pop: i32, speed: Sym) -> f64 {
    let n = (pop - 1) as f64;
    let base = r.param("CITY_GROWTH_THRESHOLD") + r.param("CITY_GROWTH_MULTIPLIER") * n + n.powf(r.param("CITY_GROWTH_EXPONENT"));
    (base * r.game_speed_cost_multiplier(speed)).floor()
}

fn plots_of(c: &City) -> std::collections::HashMap<XY, &Plot> {
    let mut m = std::collections::HashMap::new();
    for p in c.near.iter().chain(c.owned.iter()) {
        m.insert(p.xy(), p);
    }
    m
}

pub fn has_fresh_water(c: &City) -> bool {
    let plots = plots_of(c);
    let center = plots.get(&c.center);
    center.map(|p| p.river).unwrap_or(false)
        || neighbour_xy(c.center.0, c.center.1).iter().filter_map(|xy| plots.get(xy)).any(|p| p.lake || p.feature == Some(s!("FEATURE_OASIS")))
}

pub fn is_coastal(c: &City) -> bool {
    let plots = plots_of(c);
    neighbour_xy(c.center.0, c.center.1).iter().filter_map(|xy| plots.get(xy)).any(|p| p.terrain == s!("TERRAIN_COAST") && !p.lake)
}

pub fn housing(r: &Rules, c: &City, e: Option<&Engine>) -> f64 {
    let mut fresh = has_fresh_water(c);
    if let Some(e) = e {
        if e.city_effects(s!("EFFECT_ADJUST_CITIES_FRESHWATER_HOUSING_BONUS"), c.name).into_iter().chain(e.player_effects(s!("EFFECT_ADJUST_CITIES_FRESHWATER_HOUSING_BONUS")))
            .any(|x| x.args.is("HasBonus", &["1", "true"])) {
            fresh = true;
        }
    }
    let no_water = e.map(|e| e.city_effects(s!("EFFECT_ADJUST_NO_FRESH_WATER_HOUSING"), c.name).iter().any(|x| x.args.is("NoHousing", &["1", "true"]))).unwrap_or(false);
    let mut h = if no_water {
        r.param("CITY_POPULATION_NO_WATER")
    } else if fresh {
        r.param("CITY_POPULATION_RIVER_LAKE")
    } else if is_coastal(c) {
        r.param("CITY_POPULATION_COAST")
    } else {
        r.param("CITY_POPULATION_NO_WATER")
    };
    let done: Vec<Sym> = c.districts.iter().filter(|d| d.complete).map(|d| d.dtype).collect();
    if done.contains(&s!("DISTRICT_AQUEDUCT")) || done.contains(&s!("DISTRICT_BATH")) {
        h = if fresh { h + 2.0 } else { h.max(6.0) };
    }
    for d in &c.districts {
        if d.complete {
            h += r.lookup("Districts", "DistrictType", d.dtype, "Housing").map(|v| v.f()).unwrap_or(0.0);
        }
    }
    for &b in &c.buildings {
        h += r.lookup_f("Buildings", "BuildingType", b, "Housing");
    }
    for p in &c.owned {
        if let Some(i) = p.improvement {
            if !p.pillaged {
                let hs = r.lookup_f("Improvements", "ImprovementType", i, "Housing");
                if hs != 0.0 {
                    let tr = r.lookup_f("Improvements", "ImprovementType", i, "TilesRequired").max(1.0);
                    h += hs / tr;
                }
            }
        }
    }
    if let Some(e) = e {
        let n = c.name;
        for effect in ["EFFECT_ADJUST_POLICY_HOUSING", "EFFECT_ADJUST_BUILDING_HOUSING", "EFFECT_ADJUST_CITY_HOUSING_FROM_GREAT_PEOPLE"] {
            h += e.city_effects(crate::sym::sym_static(effect), n).iter().map(|x| x.args.f("Amount")).sum::<f64>();
        }
        for effect in ["EFFECT_ADJUST_DISTRICT_HOUSING", "EFFECT_ADJUST_IMPROVEMENT_HOUSING"] {
            h += e.district_effects(crate::sym::sym_static(effect), n).iter().map(|(_, x)| x.args.f("Amount")).sum::<f64>();
            h += e.owned_plot_effects(crate::sym::sym_static(effect), n).iter().map(|x| x.args.f("Amount")).sum::<f64>();
        }
        if !fresh {
            h += e.city_effects(s!("EFFECT_ADJUST_WATER_HOUSING"), n).iter().map(|x| x.args.f("Amount")).sum::<f64>();
        }
    }
    h
}

fn housing_growth_multiplier(r: &Rules, left: f64) -> f64 {
    if left <= r.param("CITY_HOUSING_LEFT_ZERO_GROWTH") {
        0.0
    } else if left <= r.param("CITY_HOUSING_LEFT_25PCT_GROWTH") {
        0.25
    } else if left <= r.param("CITY_HOUSING_LEFT_50PCT_GROWTH") {
        0.5
    } else {
        1.0
    }
}

/// Food stored this turn. `e` is the engine of the player's cities (growth_percent / housing modifiers).
pub fn food_added(r: &Rules, c: &City, pop: i32, food: f64, growth_pct: f64, e: Option<&Engine>) -> f64 {
    let surplus = food - r.param("CITY_FOOD_CONSUMPTION_PER_POPULATION") * pop as f64;
    if surplus <= 0.0 {
        return surplus;
    }
    let mut cc = c.clone();
    cc.population = pop;
    // the game compares whole housing (5.5 counts as 5)
    let left = housing(r, &cc, e).floor() - pop as f64;
    let g = growth_pct + e.map(|e| e.city_effects(s!("EFFECT_ADJUST_CITY_GROWTH"), c.name).iter().map(|x| x.args.f("Amount")).sum::<f64>()).unwrap_or(0.0);
    // the game works in fixed point: the growth bonus counts in 1/64 (8% grows a 12 surplus by 12.9375, 12 x 69/64)
    let g = (g / 100.0 * 64.0).floor() / 64.0;
    surplus * (1.0 + g) * housing_growth_multiplier(r, left)
}
