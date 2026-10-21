//! Amenities and happiness (civ6sim/amenities.py).
use crate::engine::{ss, Engine};
use crate::geo::neighbour_xy;
use crate::model::*;
use crate::rules::Rules;
use crate::sym::Sym;

fn improved_luxuries(r: &Rules, cities: &[City], p: &Player) -> Vec<(Sym, i64)> {
    let mut out: Vec<(Sym, i64)> = vec![];
    let mut add = |k: Sym| if let Some(x) = out.iter_mut().find(|(a, _)| *a == k) { x.1 += 1 } else { out.push((k, 1)) };
    for e in &p.envoys {
        if e.suzerain {
            for &res in &e.resources {
                if r.resource_class(res) == Some(s!("RESOURCECLASS_LUXURY")) {
                    add(res);
                }
            }
        }
    }
    for c in cities {
        for p in &c.owned {
            if let Some(res) = p.resource {
                if r.resource_class(res) == Some(s!("RESOURCECLASS_LUXURY")) && !p.pillaged
                    && p.improvement.map(|i| r.valid_improvements(res).contains(&i)).unwrap_or(false) {
                    add(res);
                }
            }
        }
    }
    out
}

pub fn amenities_needed(r: &Rules, pop: i32) -> f64 {
    ((pop as f64 / r.param("CITY_POP_PER_AMENITY")).ceil() - r.param("CITY_AMENITIES_FOR_FREE").trunc()).max(0.0)
}

/// City name -> amenity score (supply minus needed), for cities at their simulated populations. `extra`: luxury types
/// from outside the player's land (each one's amenities go to the neediest cities, as an own luxury's do).
pub fn amenity_scores(r: &Rules, cities: &[City], e: &Engine, extra: i64) -> Vec<(Sym, f64)> {
    let _g = crate::prof::start(6);
    let mut supply: Vec<(Sym, f64)> = cities.iter().map(|c| (c.name, 0.0)).collect();
    let sup = |supply: &mut Vec<(Sym, f64)>, n: Sym, v: f64| supply.iter_mut().find(|(a, _)| *a == n).unwrap().1 += v;
    for c in cities {
        for &b in &c.buildings {
            sup(&mut supply, c.name, r.lookup_f("Buildings", "BuildingType", b, "Entertainment"));
        }
        for d in &c.districts {
            if d.complete {
                sup(&mut supply, c.name, r.lookup("Districts", "DistrictType", d.dtype, "Entertainment").map(|v| v.f()).unwrap_or(0.0));
            }
        }
    }
    for c in cities {
        let n = c.name;
        for effect in ["EFFECT_ADJUST_POLICY_AMENITY", "EFFECT_ADJUST_TRAIT_AMENITY", "EFFECT_ADJUST_IMPROVEMENT_AMENITY",
            "EFFECT_ADJUST_CITY_AMENITIES_FROM_GOVERNORS", "EFFECT_ADJUST_CITY_AMENITIES_FROM_RELIGION",
            "EFFECT_ADJUST_CITY_AMENITIES_FROM_CITY_STATES", "EFFECT_ADJUST_CITY_AMENITIES_FROM_GREAT_PEOPLE"] {
            for x in e.city_effects(crate::sym::sym_static(effect), n) {
                sup(&mut supply, n, x.args.f("Amount"));
            }
        }
        for effect in ["EFFECT_ADJUST_DISTRICT_AMENITY", "EFFECT_ADJUST_IMPROVEMENT_AMENITY"] {
            for (_, x) in e.district_effects(crate::sym::sym_static(effect), n) {
                sup(&mut supply, n, x.args.f("Amount"));
            }
            for x in e.owned_plot_effects(crate::sym::sym_static(effect), n) {
                sup(&mut supply, n, x.args.f("Amount"));
            }
        }
        for effect in ["EFFECT_ADJUST_CITY_ENTERTAINMENT", "EFFECT_ADJUST_NATURAL_WONDER_AMENITY"] {
            for x in e.city_effects(crate::sym::sym_static(effect), n) {
                sup(&mut supply, n, x.args.f("Amount"));
            }
        }
        for (d, x) in e.district_effects(s!("EFFECT_ADJUST_DISTRICT_EXTRA_ENTERTAINMENT"), n) {
            if d.map(|d| d.complete).unwrap_or(true) {
                sup(&mut supply, n, x.args.f("Amount"));
            }
        }
        for x in e.city_effects(s!("EFFECT_ADJUST_CITY_ENTERTAINMENT_FROM_WONDER_ADJACENT_TO_LAKE"), n) {
            if let Some(b) = x.args.get("BuildingType") {
                if c.buildings.contains(&b) {
                    for (xy, q) in e.plots.iter() {
                        if q.wonder == Some(b) {
                            let k = neighbour_xy(xy.0, xy.1).iter().filter(|n2| e.plots.get(n2).map(|p| p.lake).unwrap_or(false)).count();
                            sup(&mut supply, n, k as f64);
                        }
                    }
                }
            }
        }
        let lux_here = c.owned.iter().filter(|p| p.resource.map(|res| r.resource_class(res) == Some(s!("RESOURCECLASS_LUXURY")) && !p.pillaged
            && p.improvement.map(|i| r.valid_improvements(res).contains(&i)).unwrap_or(false)).unwrap_or(false)).count();
        for x in e.city_effects(s!("EFFECT_ADJUST_CITY_EXTRA_AMENITY_FOR_LUXURY_DIVERSITY"), n) {
            sup(&mut supply, n, x.args.f("Amount") * lux_here as f64);
        }
        for x in e.city_effects(s!("EFFECT_ADJUST_ADD_AMENITY_PER_ADJACENT_LUXURY"), n) {
            let near = neighbour_xy(c.center.0, c.center.1);
            let k = c.owned.iter().filter(|p| near.contains(&p.xy()) && p.resource.map(|res| r.resource_class(res) == Some(s!("RESOURCECLASS_LUXURY"))).unwrap_or(false)).count();
            let per = x.args.get("AddAmenity").map(|_| x.args.f("AddAmenity")).unwrap_or(1.0);
            sup(&mut supply, n, per * k as f64);
        }
    }
    let need: Vec<f64> = cities.iter().map(|c| amenities_needed(r, c.population)).collect();
    let mut per_lux = 4i64;
    let mut types = improved_luxuries(r, cities, &e.env.p);
    per_lux += e.player_effects(s!("EFFECT_ADJUST_OWNED_LUXURY_EXTRA_AMENITIES")).iter().map(|x| x.args.f("Amount") as i64).sum::<i64>();
    if !e.player_effects(s!("EFFECT_ADJUST_OWNED_BONUS_RESOURCE_EXTRA_AMENITIES")).is_empty() {
        for c in cities {
            for p in &c.owned {
                if let Some(res) = p.resource {
                    if r.resource_class(res) == Some(s!("RESOURCECLASS_BONUS")) && !p.pillaged
                        && p.improvement.map(|i| r.valid_improvements(res).contains(&i)).unwrap_or(false) {
                        if let Some(x) = types.iter_mut().find(|(a, _)| *a == res) { x.1 += 1 } else { types.push((res, 1)) }
                    }
                }
            }
        }
    }
    // replay: the luxury amenities as the save gives them to each city
    if let Some(given) = &e.env.luxuries_given {
        for (n, v) in given {
            if let Some(x) = supply.iter_mut().find(|(a, _)| a == n) {
                x.1 += v;
            }
        }
        types.clear();
    }
    let extra = if e.env.luxuries_given.is_some() { 0 } else { extra };
    for _ in 0..(types.len() + extra.max(0) as usize) {
        let mut order: Vec<usize> = (0..cities.len()).collect();
        // sorted by supply - need (stable, like Python's sort)
        order.sort_by(|&a, &b| (supply[a].1 - need[a]).partial_cmp(&(supply[b].1 - need[b])).unwrap());
        for &i in order.iter().take(per_lux as usize) {
            supply[i].1 += 1.0;
        }
    }
    if crate::envcfg::trace() == Some("amen") {
        eprintln!("AMEN lux {:?} extra {extra} {:?}", types.iter().map(|(r, n)| (r.as_string(), *n)).collect::<Vec<_>>(), cities.iter().enumerate().map(|(i, c)| (c.name.as_string(), supply[i].1, need[i])).collect::<Vec<_>>());
    }
    cities.iter().enumerate().map(|(i, c)| (c.name, supply[i].1 - need[i])).collect()
}

/// (non-food yield %, growth %) for an amenity score.
pub fn happiness(r: &Rules, score: f64) -> (f64, f64) {
    let t = r.tab("Happinesses");
    for i in 0..t.len() as u32 {
        let lo = t.v(i, "MinimumAmenityScore");
        let hi = t.v(i, "MaximumAmenityScore");
        if (lo.is_null() || score >= lo.f()) && (hi.is_null() || score <= hi.f()) {
            return (t.v(i, "NonFoodYieldModifier").f(), t.v(i, "GrowthModifier").f());
        }
    }
    (0.0, 0.0)
}
