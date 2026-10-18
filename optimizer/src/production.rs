//! Production: costs, percents, purchases, maintenance, harvests, slots (civ6sim/production.py).
use crate::engine::{ss, Engine};
use crate::geo::{hex_distance, neighbour_xy};
use crate::gp::government_bonus;
use crate::model::*;
use crate::rules::Rules;
use crate::sym::Sym;
use std::collections::BTreeSet;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Kind {
    Unit,
    Building,
    District,
    Project,
    Other,
}

pub fn item_kind(item: Sym) -> Kind {
    item.with(|s| match s.split('_').next().unwrap_or("") {
        "UNIT" => Kind::Unit,
        "BUILDING" => Kind::Building,
        "DISTRICT" => Kind::District,
        "PROJECT" => Kind::Project,
        _ => Kind::Other,
    })
}

pub fn game_progress(r: &Rules, techs: usize, civics: usize) -> f64 {
    (100.0 * (techs as f64 / r.n_tech as f64).max(civics as f64 / r.n_civic as f64)).floor()
}

fn game_progress_cost(r: &Rules, base: f64, param: f64, techs: usize, civics: usize) -> f64 {
    base * (1.0 + (param / 100.0 - 1.0) * game_progress(r, techs, civics) / 100.0)
}

/// Harvest yields (production.harvest_yield).
pub fn harvest_yield(r: &Rules, e: Option<&Engine>, kind_feature: bool, what: Sym, techs: usize, civics: usize, city: Option<Sym>, speed: Sym) -> Vec<(Sym, f64)> {
    let rows: Vec<(Sym, f64)> = if kind_feature {
        r.yield_rows("Feature_Removes", "FeatureType", what, "YieldType", "Yield")
    } else {
        r.yield_rows("Resource_Harvests", "ResourceType", what, "YieldType", "Amount")
    };
    let p = game_progress(r, techs, civics) / 100.0;
    let bonus: f64 = match (e, city) {
        (Some(e), Some(c)) => e.city_effects(s!("EFFECT_ADJUST_CITY_RESOURCE_HARVEST_BONUS"), c).iter().map(|x| x.args.f("Amount")).sum(),
        _ => 0.0,
    };
    let m = r.game_speed_cost_multiplier(speed);
    let mut out: Vec<(Sym, f64)> = vec![];
    for (k, v) in rows {
        let val = (v * (1.0 + 9.0 * p) * m * (1.0 + bonus / 100.0)).floor();
        if let Some(x) = out.iter_mut().find(|(a, _)| *a == k) { x.1 = val } else { out.push((k, val)) }
    }
    out
}

pub fn gold_price(exact_cost: f64, discount: f64) -> f64 {
    (4.0 * exact_cost * (1.0 - discount / 100.0) / 5.0).floor() * 5.0
}
pub fn faith_price(exact_cost: f64, discount: f64) -> f64 {
    (2.0 * exact_cost * (1.0 - discount / 100.0) / 5.0).floor() * 5.0
}

pub fn purchase_yields(r: &Rules, e: &Engine, c: &City, item: Sym) -> Vec<Sym> {
    let eff = |x: &'static str| -> Vec<crate::rules::Args> { e.city_effects(crate::sym::sym_static(x), c.name).into_iter().chain(e.player_effects(crate::sym::sym_static(x))).map(|x| x.args.clone()).collect() };
    let mut out: Vec<Sym> = vec![];
    match item_kind(item) {
        Kind::Unit => {
            if let Some(py) = r.lookup_sym("Units", "UnitType", item, "PurchaseYield") {
                out.push(py);
            }
            let tags = r.tags_of(Some(item));
            if eff("EFFECT_ENABLE_UNIT_FAITH_PURCHASE").iter().any(|a| a.get("Tag").map(|t| tags.contains(&t)).unwrap_or(false)) {
                out.push(s!("YIELD_FAITH"));
            }
        }
        Kind::Building => {
            if let Some(py) = r.lookup_sym("Buildings", "BuildingType", item, "PurchaseYield") {
                out.push(py);
            }
            let pd = r.lookup_sym("Buildings", "BuildingType", item, "PrereqDistrict");
            let districts: Vec<Sym> = match pd {
                Some(d) => {
                    let mut v = vec![d];
                    v.extend(r.list_sym("DistrictReplaces", "ReplacesDistrictType", d, "CivUniqueDistrictType"));
                    v
                }
                None => vec![s!("DISTRICT_CITY_CENTER")],
            };
            if eff("EFFECT_ENABLE_BUILDING_FAITH_PURCHASE").iter().any(|a| a.get("DistrictType").map(|d| districts.contains(&d)).unwrap_or(false))
                || eff("EFFECT_ENABLE_SPECIFIC_BUILDING_FAITH_PURCHASE").iter().any(|a| a.get("BuildingType") == Some(item)) {
                out.push(s!("YIELD_FAITH"));
            }
        }
        Kind::District => {
            if eff("EFFECT_ADJUST_CITY_CAN_PURCHASE_DISTRICTS").iter().any(|a| a.is("CanPurchase", &["1", "true"])) {
                out.push(s!("YIELD_GOLD"));
            }
            if eff("EFFECT_ADJUST_CAN_FAITH_PURCHASE_DISTRICTS").iter().any(|a| a.is("CanPurchase", &["1", "true"])) {
                out.push(s!("YIELD_FAITH"));
            }
        }
        _ => {}
    }
    let mut dedup = vec![];
    for x in out {
        if !dedup.contains(&x) {
            dedup.push(x);
        }
    }
    dedup
}

/// Unit cost; None for cost models not calibrated.
pub fn unit_cost(r: &Rules, env: &Env, unit: Sym, speed: Sym, exact: bool) -> Option<f64> {
    let t = r.tab("Units");
    let i = t.first("UnitType", unit)?;
    let mut cost = t.v(i, "Cost").f();
    let model = t.v(i, "CostProgressionModel").sym();
    let param = t.v(i, "CostProgressionParam1").f();
    let m = r.game_speed_cost_multiplier(speed);
    if model == Some(s!("COST_PROGRESSION_PREVIOUS_COPIES")) {
        cost += param * *env.units_trained.get(&unit).unwrap_or(&0) as f64;
    } else if model == Some(s!("NO_COST_PROGRESSION")) {
        return Some((cost * m).trunc());
    } else if model == Some(s!("COST_PROGRESSION_GAME_PROGRESS")) {
        let c = game_progress_cost(r, cost, param, env.techs.len(), env.civics.len()) * m;
        return Some(if exact { c } else { c.floor() });
    } else {
        return None;
    }
    Some((cost * m).trunc())
}

pub fn building_cost(r: &Rules, b: Sym, speed: Sym, exact: bool) -> f64 {
    let c = r.lookup_f("Buildings", "BuildingType", b, "Cost") * r.game_speed_cost_multiplier(speed);
    if exact { c } else { c.trunc() }
}

pub fn unlocked_districts(r: &Rules, techs: &BTreeSet<Sym>, civics: &BTreeSet<Sym>) -> usize {
    let t = r.tab("Districts");
    let mut n = 0;
    for i in 0..t.len() as u32 {
        if !t.v(i, "RequiresPopulation").truthy() || t.v(i, "DistrictType").sym() == Some(s!("DISTRICT_CITY_CENTER")) {
            continue;
        }
        if t.v(i, "TraitType").sym().map(|s| !s.is_empty()).unwrap_or(false) {
            continue;
        }
        let pt = t.v(i, "PrereqTech").sym().filter(|s| !s.is_empty());
        let pc = t.v(i, "PrereqCivic").sym().filter(|s| !s.is_empty());
        if pt.map(|x| techs.contains(&x)).unwrap_or(true) && pc.map(|x| civics.contains(&x)).unwrap_or(true) {
            n += 1;
        }
    }
    n
}

pub fn district_cost(r: &Rules, d: Sym, speed: Sym, env: &Env, placed: Option<&[(Sym, i64)]>, exact: bool) -> f64 {
    let t = r.tab("Districts");
    let i = t.first("DistrictType", d).unwrap();
    let base = t.v(i, "Cost").f();
    let param = t.v(i, "CostProgressionParam1").f();
    let model = t.v(i, "CostProgressionModel").sym();
    let m = r.game_speed_cost_multiplier(speed);
    if model == Some(s!("COST_PROGRESSION_GAME_PROGRESS")) {
        let c = game_progress_cost(r, base, param, env.techs.len(), env.civics.len()) * m;
        return if exact { c } else { c.floor() };
    }
    if model != Some(s!("COST_PROGRESSION_NUM_UNDER_AVG_PLUS_TECH")) {
        let c = base * m;
        return if exact { c } else { c.floor() };
    }
    let progress = (100.0 * (env.techs.len() as f64 / r.n_tech as f64).max(env.civics.len() as f64 / r.n_civic as f64)).floor() / 100.0;
    let mut f = 1.0;
    if let Some(placed) = placed {
        let a = unlocked_districts(r, &env.techs, &env.civics);
        if a > 0 {
            let b = env.districts_completed.unwrap_or(0) as f64;
            let cnt = placed.iter().find(|(k, _)| *k == d).map(|(_, v)| *v).unwrap_or(0) as f64;
            if b >= a as f64 && b / a as f64 > cnt {
                f = 1.0 - param / 100.0;
            }
        }
    }
    let cost = (base * f).floor() * (1.0 + 9.0 * progress) * m;
    if exact { cost } else { cost.floor() }
}

pub fn project_cost(r: &Rules, env: &Env, p: Sym, speed: Sym, exact: bool) -> Option<f64> {
    let t = r.tab("Projects");
    let i = t.first("ProjectType", p)?;
    let cost = t.v(i, "Cost").f();
    let model = t.v(i, "CostProgressionModel").sym();
    let param = t.v(i, "CostProgressionParam1").f();
    let m = r.game_speed_cost_multiplier(speed);
    let c = if model == Some(s!("COST_PROGRESSION_GAME_PROGRESS")) {
        game_progress_cost(r, cost, param, env.techs.len(), env.civics.len()) * m
    } else {
        cost * m
    };
    Some(if exact { c } else { c.floor() })
}

pub fn item_cost(r: &Rules, env: &Env, item: Sym, speed: Sym, placed: Option<&[(Sym, i64)]>, exact: bool) -> Option<f64> {
    match item_kind(item) {
        Kind::Unit => unit_cost(r, env, item, speed, exact),
        Kind::Building => Some(building_cost(r, item, speed, exact)),
        Kind::District => Some(district_cost(r, item, speed, env, placed, exact)),
        Kind::Project => project_cost(r, env, item, speed, exact),
        Kind::Other => None,
    }
}

pub fn builder_charges(r: &Rules, e: &Engine, unit: Sym, city: Sym) -> i64 {
    r.lookup_f("Units", "UnitType", unit, "BuildCharges") as i64
        + e.unit_effects(s!("EFFECT_ADJUST_UNIT_BUILD_CHARGES"), unit, Some(city)).iter().map(|x| x.args.f("Amount") as i64).sum::<i64>()
}

fn era_index(r: &Rules, era: Option<Sym>) -> Option<usize> {
    era.and_then(|e| r.era_index(e))
}

fn item_plot(e: &Engine, c: &City, item: Sym) -> Option<Plot> {
    if item_kind(item) == Kind::District {
        return c.districts.iter().find(|x| x.dtype == item).map(|d| d.plot.clone());
    }
    e.plots.values().find(|q| q.wonder == Some(item)).cloned()
}

fn center_river(e: &Engine, c: &City) -> bool {
    e.plots.get(&c.center).map(|p| p.river).unwrap_or(false)
}

fn placement_percent(e: &Engine, c: &City, plot: Option<&Plot>, kind: &str) -> f64 {
    let Some(p) = plot else { return 0.0 };
    let mut pct = 0.0;
    if p.river {
        for x in e.city_effects(crate::engine::ss(&format!("EFFECT_ADJUST_RIVER_{kind}_PRODUCTION")), c.name) {
            pct += x.args.f("Amount");
        }
    }
    let near: Vec<Option<Sym>> = neighbour_xy(p.x, p.y).iter().map(|xy| e.plots.get(xy).and_then(|q| q.feature)).collect();
    for x in e.city_effects(crate::engine::ss(&format!("EFFECT_ADJUST_{kind}_ADJACENT_NATURAL_WONDER_PRODUCTION")), c.name) {
        if near.contains(&x.args.get("FeatureType")) {
            pct += x.args.f("Amount");
        }
    }
    pct
}

pub fn item_production_percent(r: &Rules, e: &Engine, c: &City, item: Sym) -> f64 {
    let n = c.name;
    let mut pct = 0.0;
    let eff = |x: &'static str| e.city_effects(crate::sym::sym_static(x), n);
    match item_kind(item) {
        Kind::Unit => {
            let cls = r.lookup_sym("Units", "UnitType", item, "PromotionClass");
            let domain = r.lookup_sym("Units", "UnitType", item, "Domain");
            let era = r.unit_era(item);
            let military = cls.is_some() && ![s!("UNIT_BUILDER"), s!("UNIT_SETTLER"), s!("UNIT_TRADER")].contains(&item);
            for x in eff("EFFECT_ADJUST_UNIT_PRODUCTION").into_iter().chain(e.player_effects(s!("EFFECT_ADJUST_UNIT_PRODUCTION")))
                .chain(e.unit_effects(s!("EFFECT_ADJUST_UNIT_PRODUCTION"), item, Some(n))) {
                if x.args.get("UnitType") == Some(item) {
                    pct += x.args.f("Amount");
                }
            }
            for x in eff("EFFECT_ADJUST_UNIT_TAG_ERA_PRODUCTION") {
                if x.args.get("UnitPromotionClass") == cls && x.args.get("EraType") == Some(era) {
                    pct += x.args.f("Amount");
                }
            }
            for x in eff("EFFECT_ADJUST_ALL_UNIT_PRODUCTION_MODIFIER") {
                pct += x.args.f("Amount");
            }
            for x in eff("EFFECT_ADJUST_UNIT_DOMAIN_PRODUCTION") {
                if x.args.get("Domain") == domain {
                    pct += x.args.f("Amount");
                }
            }
            if military {
                pct += government_bonus(e, "GOVERNMENTBONUS_UNIT_PRODUCTION");
                let ei = r.era_index(era);
                for x in eff("EFFECT_ADJUST_CITY_ALL_MILITARY_UNITS_PRODUCTION") {
                    let lo = era_index(r, x.args.get("StartEra"));
                    let hi = era_index(r, x.args.get("EndEra"));
                    if lo.map(|l| ei.unwrap_or(0) >= l).unwrap_or(true) && hi.map(|h| ei.unwrap_or(0) <= h).unwrap_or(true)
                        && (x.args.get("PromotionClass").is_none() || x.args.get("PromotionClass") == cls) {
                        pct += x.args.f("Amount");
                    }
                }
            }
        }
        Kind::Building => {
            let wonder = r.is_wonder(item);
            let district = r.lookup_sym("Buildings", "BuildingType", item, "PrereqDistrict");
            for x in eff("EFFECT_ADJUST_BUILDING_PRODUCTION") {
                if x.args.get("BuildingType") == Some(item) || (x.args.get("DistrictType").map(|d| !d.is_empty()).unwrap_or(false) && x.args.get("DistrictType") == district) {
                    pct += x.args.f("Amount");
                }
            }
            for x in eff("EFFECT_ADJUST_ALL_BUILDING_PRODUCTION_MODIFIER") {
                let ok = match x.args.get("IsWonder") {
                    None => true,
                    Some(v) => v.with(|s| if wonder { s == "1" || s == "true" } else { s == "0" || s == "false" }),
                };
                if ok {
                    pct += x.args.f("Amount");
                }
            }
            if wonder {
                let era = r.lookup_sym("Buildings", "BuildingType", item, "PrereqTech").and_then(|t| r.lookup_sym("Technologies", "TechnologyType", t, "EraType"))
                    .or_else(|| r.lookup_sym("Buildings", "BuildingType", item, "PrereqCivic").and_then(|t| r.lookup_sym("Civics", "CivicType", t, "EraType")));
                let ei = era_index(r, era);
                for x in eff("EFFECT_ADJUST_WONDER_PRODUCTION") {
                    pct += x.args.f("Amount");
                }
                for x in eff("EFFECT_ADJUST_WONDER_ERA_PRODUCTION") {
                    let lo = era_index(r, x.args.get("StartEra"));
                    let hi = era_index(r, x.args.get("EndEra"));
                    if let Some(ei) = ei {
                        if lo.map(|l| ei >= l).unwrap_or(true) && hi.map(|h| ei <= h).unwrap_or(true) {
                            pct += x.args.f("Amount");
                        }
                    }
                }
                pct += government_bonus(e, "GOVERNMENTBONUS_WONDER_CONSTRUCTION");
                pct += placement_percent(e, c, item_plot(e, c, item).as_ref(), "WONDER");
            } else if district.is_some() {
                for x in eff("EFFECT_ADJUST_DISTRICT_BUILDING_PRODUCTION") {
                    pct += x.args.f("Amount");
                }
                if let Some(d) = c.districts.iter().find(|x| r.building_in_district(item, x.dtype)) {
                    if d.plot.river && center_river(e, c) {
                        for x in eff("EFFECT_ADJUST_ADJACENT_CITY_RIVER_BUILDING_PRODUCTION") {
                            pct += x.args.f("Amount");
                        }
                    }
                }
            }
        }
        Kind::District => {
            let mut base = vec![item];
            base.extend(r.list_sym("DistrictReplaces", "CivUniqueDistrictType", item, "ReplacesDistrictType"));
            for x in eff("EFFECT_ADJUST_DISTRICT_PRODUCTION") {
                if x.args.get("DistrictType").map(|d| base.contains(&d)).unwrap_or(false) {
                    pct += x.args.f("Amount");
                }
            }
            for ef in ["EFFECT_ADJUST_ALL_DISTRICT_PRODUCTION_MODIFIER", "EFFECT_ADJUST_ALL_DISTRICTS_PRODUCTION"] {
                for x in eff(ef) {
                    pct += x.args.f("Amount");
                }
            }
            pct += government_bonus(e, "GOVERNMENTBONUS_DISTRICT_PRODUCTION");
            let plot = item_plot(e, c, item);
            pct += placement_percent(e, c, plot.as_ref(), "DISTRICT");
            if plot.map(|p| p.river).unwrap_or(false) && center_river(e, c) {
                for x in eff("EFFECT_ADJUST_ADJACENT_CITY_RIVER_DISTRICT_PRODUCTION") {
                    pct += x.args.f("Amount");
                }
            }
        }
        Kind::Project => {
            for x in eff("EFFECT_ADJUST_PROJECT_PRODUCTION") {
                if x.args.get("ProjectType") == Some(item) {
                    pct += x.args.f("Amount");
                }
            }
            for x in eff("EFFECT_ADJUST_ALL_PROJECTS_PRODUCTION") {
                pct += x.args.f("Amount");
            }
        }
        Kind::Other => {}
    }
    pct
}

pub fn item_production_flat(e: &Engine, c: &City, item: Sym) -> f64 {
    let eff = match item_kind(item) {
        Kind::Unit => "EFFECT_ADJUST_CITY_PRODUCTION_UNIT",
        Kind::Building => "EFFECT_ADJUST_CITY_PRODUCTION_BUILDING",
        Kind::District => "EFFECT_ADJUST_CITY_PRODUCTION_DISTRICT",
        _ => return 0.0,
    };
    e.city_effects(crate::sym::sym_static(eff), c.name).iter().map(|x| x.args.f("Amount")).sum()
}

pub fn purchase_discount_percent(r: &Rules, e: &Engine, c: &City, item: Sym, gold: bool) -> f64 {
    let eff = |x: &'static str| -> Vec<crate::rules::Args> { e.city_effects(crate::sym::sym_static(x), c.name).into_iter().chain(e.player_effects(crate::sym::sym_static(x))).map(|x| x.args.clone()).collect() };
    let mut pct = 0.0;
    if gold {
        pct += government_bonus(e, "GOVERNMENTBONUS_GOLD_PURCHASES");
    }
    match item_kind(item) {
        Kind::Unit => {
            let domain = r.lookup_sym("Units", "UnitType", item, "Domain");
            let civilian = r.lookup_sym("Units", "UnitType", item, "PromotionClass").is_none();
            for a in eff("EFFECT_ADJUST_UNIT_PURCHASE_COST") {
                if a.get("UnitType") == Some(item) {
                    pct += a.f("Amount");
                }
            }
            for a in eff("EFFECT_ADJUST_ALL_UNITS_PURCHASE_COST") {
                if (a.get("UnitDomain").is_none() || a.get("UnitDomain") == domain) && (!civilian || a.is("IncludeCivilian", &["1", "true", "True"])) {
                    pct += a.f("Amount");
                }
            }
        }
        Kind::Building => {
            for a in eff("EFFECT_ADJUST_BUILDING_PURCHASE_COST") {
                if a.get("BuildingType") == Some(item) {
                    pct += a.f("Amount");
                }
            }
            for a in eff("EFFECT_ADJUST_ALL_BUILDINGS_PURCHASE_COST") {
                pct += a.f("Amount");
            }
        }
        Kind::District => {
            for a in eff("EFFECT_ADJUST_ALL_DISTRICTS_PURCHASE_COST") {
                pct += a.f("Amount");
            }
        }
        _ => {}
    }
    pct
}

pub fn faith_discount_percent(r: &Rules, e: &Engine, c: &City, item: Sym) -> f64 {
    let mut pct = purchase_discount_percent(r, e, c, item, false) + government_bonus(e, "GOVERNMENTBONUS_FAITH_PURCHASES");
    if item_kind(item) == Kind::Building && r.lookup_truthy("Buildings", "BuildingType", item, "EnabledByReligion") {
        for x in e.player_effects(s!("EFFECT_ADJUST_RELIGION_BUILDING_DISCOUNT")) {
            pct += x.args.f("Discount");
        }
    }
    pct.min(100.0)
}

pub fn maintenance(r: &Rules, e: &Engine, cities: &[City], units: &OMap<Sym, i64>) -> f64 {
    let mut total = 0.0;
    for c in cities {
        for &b in &c.buildings {
            total += r.lookup("Buildings", "BuildingType", b, "Maintenance").map(|v| v.f()).unwrap_or(0.0);
        }
        for d in &c.districts {
            if d.complete {
                total += r.lookup("Districts", "DistrictType", d.dtype, "Maintenance").map(|v| v.f()).unwrap_or(0.0);
            }
        }
    }
    let discount: f64 = e.player_effects(s!("EFFECT_ADJUST_UNIT_MAINTENANCE_DISCOUNT")).iter().map(|x| x.args.f("Amount")).sum();
    for (u, n) in units.iter() {
        let m = r.lookup("Units", "UnitType", *u, "Maintenance").map(|v| v.f()).unwrap_or(0.0);
        if m != 0.0 {
            total += *n as f64 * (m - discount).max(0.0);
        }
    }
    total
}

pub fn trade_route_capacity(e: &Engine) -> i64 {
    let cap: f64 = e.all_effects(s!("EFFECT_ADJUST_TRADE_ROUTE_CAPACITY")).iter().map(|x| x.args.f("Amount")).sum();
    cap.trunc() as i64
}

pub fn policy_slots(r: &Rules, e: &Engine, government: Sym) -> Vec<Sym> {
    let t = r.tab("Government_SlotCounts");
    let mut slots: Vec<Sym> = vec![];
    for &i in t.find("GovernmentType", government) {
        for _ in 0..t.v(i, "NumSlots").i() {
            slots.push(t.v(i, "GovernmentSlotType").sym().unwrap());
        }
    }
    for x in e.all_effects(s!("EFFECT_ADJUST_PLAYER_GOVERNMENT_SLOT_TYPE")) {
        slots.push(x.args.get("GovernmentSlotType").unwrap());
    }
    for x in e.all_effects(s!("EFFECT_ADJUST_GOVERNMENT_SLOTS")) {
        let n = x.args.f("Amount") as i64;
        let st = x.args.get("GovernmentSlotType").unwrap();
        for _ in 0..n.abs() {
            if n > 0 {
                slots.push(st);
            } else if let Some(i) = slots.iter().position(|&s| s == st) {
                slots.remove(i);
            }
        }
    }
    for x in e.all_effects(s!("EFFECT_REPLACE_PLAYER_GOVERNMENT_SLOT_TYPE")) {
        let old = x.args.get("ReplacedGovernmentSlotType").unwrap();
        let new = x.args.get("AddedGovernmentSlotType").unwrap();
        if x.args.is("ReplacesAll", &["1", "true", "True"]) {
            for s in slots.iter_mut() {
                if *s == old {
                    *s = new;
                }
            }
        } else if let Some(i) = slots.iter().position(|&s| s == old) {
            slots[i] = new;
        }
    }
    slots
}

pub fn tile_price(r: &Rules, e: &Engine, c: &City, p: &Plot, speed: Sym, techs: usize, civics: usize) -> f64 {
    let d = hex_distance(c.center, p.xy());
    let base = if d <= 2 { 50.0 } else { 75.0 };
    let mut pct: f64 = e.city_effects(s!("EFFECT_ADJUST_PLOT_PURCHASE_COST"), c.name).iter().map(|x| x.args.f("Amount")).sum();
    pct += e.city_effects(s!("EFFECT_ADJUST_PLOT_PURCHASE_COST_TERRAIN"), c.name).iter().filter(|x| x.args.get("TerrainType") == Some(p.terrain)).map(|x| x.args.f("Amount")).sum::<f64>();
    let v = base * (1.0 + 4.0 * game_progress(r, techs, civics) / 100.0) * r.game_speed_cost_multiplier(speed);
    let v6 = (v * 1e6).round() / 1e6;
    (5.0 * (v6 / 5.0).floor() * (1.0 + pct / 100.0)).floor()
}
