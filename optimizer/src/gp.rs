//! Great person points (civ6sim/greatpeople.py).
use crate::engine::Engine;
use crate::rules::Rules;
use crate::sym::Sym;
use crate::yields::push_y;

pub fn government_bonus(e: &Engine, bonus: &str) -> f64 {
    let b = crate::sym::sym(bonus);
    e.player_effects(s!("EFFECT_ADJUST_FLAT_BONUS")).iter().filter(|x| x.args.get("BonusType") == Some(b)).map(|x| x.args.f("Amount")).sum()
}

pub fn great_person_points(r: &Rules, e: &Engine) -> Vec<(Sym, f64)> {
    let mut total: Vec<(Sym, f64)> = vec![];
    if !e.player_effects(s!("EFFECT_ADJUST_NO_GREAT_PERSON_POINTS")).is_empty() {
        return total;
    }
    let gov = government_bonus(e, "GOVERNMENTBONUS_GREAT_PEOPLE");
    let mut cls_pct: Vec<(Sym, f64)> = vec![];
    for x in e.player_effects(s!("EFFECT_ADJUST_GREAT_PERSON_POINTS_PERCENT")) {
        push_y(&mut cls_pct, x.args.get("GreatPersonClassType").unwrap(), x.args.f("Amount"));
    }
    let cp = |c: Sym| cls_pct.iter().find(|(a, _)| *a == c).map(|(_, v)| *v).unwrap_or(0.0);
    for c in &e.cities {
        let mut pts: Vec<(Sym, f64)> = vec![];
        for &b in &c.buildings {
            for (k, v) in r.yield_rows("Building_GreatPersonPoints", "BuildingType", b, "GreatPersonClassType", "PointsPerTurn") {
                push_y(&mut pts, k, v);
            }
        }
        for d in &c.districts {
            if d.complete {
                for (k, v) in r.yield_rows("District_GreatPersonPoints", "DistrictType", d.dtype, "GreatPersonClassType", "PointsPerTurn") {
                    push_y(&mut pts, k, v);
                }
            }
        }
        for (d, x) in e.district_effects(s!("EFFECT_ADJUST_DISTRICT_GREAT_PERSON_POINTS"), c.name) {
            if d.map(|d| d.complete).unwrap_or(true) {
                push_y(&mut pts, x.args.get("GreatPersonClassType").unwrap(), x.args.f("Amount"));
            }
        }
        for x in e.city_effects(s!("EFFECT_ADJUST_GREAT_PERSON_POINTS"), c.name) {
            push_y(&mut pts, x.args.get("GreatPersonClassType").unwrap(), x.args.f("Amount"));
        }
        let pct: f64 = e.city_effects(s!("EFFECT_ADJUST_CITY_GREAT_PERSON_POINTS_MODIFIER"), c.name).iter().map(|x| x.args.f("Amount")).sum();
        for (cls, v) in pts {
            push_y(&mut total, cls, (v * (1.0 + (pct + gov + cp(cls)) / 100.0) * 256.0).floor() / 256.0);
        }
    }
    let mut flat: Vec<(Sym, f64)> = vec![];
    for x in e.player_effects(s!("EFFECT_ADJUST_GREAT_PERSON_POINTS")) {
        push_y(&mut flat, x.args.get("GreatPersonClassType").unwrap(), x.args.f("Amount"));
    }
    for (cls, v) in flat {
        push_y(&mut total, cls, (v * (1.0 + (gov + cp(cls)) / 100.0) * 256.0).floor() / 256.0);
    }
    total
}

/// Faith from prophet points once no great prophet is on offer (the plan never recruits one).
pub fn prophet_faith(r: &Rules, e: &Engine) -> f64 {
    if e.env.prophet_on_offer {
        return 0.0;
    }
    let p = crate::sym::sym("GREAT_PERSON_CLASS_PROPHET");
    great_person_points(r, e).iter().find(|(k, _)| *k == p).map(|(_, v)| *v).unwrap_or(0.0) * r.param("FAITH_PER_UNUSED_GREAT_PERSON_POINT")
}
