//! Strategic resources (Gathering Storm): a unit that needs one (Units.StrategicResource, Units_XP2.ResourceCost:
//! 20 iron for a knight) takes that much from the player's stockpile when it is completed or bought. The stockpile
//! gains, each turn, the extraction of every owned plot with the resource revealed (Resources.PrereqTech): improved
//! with a valid improvement (Improvement_ValidResources) ImprovedExtractionRate, else BaseExtractionRate
//! (Resource_Consumption), and what modifiers grant (BBG: barracks 2 iron, stables 2 horses, Equestrian Orders, ...:
//! EFFECT_ADJUST_PLAYER_FREE_RESOURCE_IMPORT_EXTRACTION and the free-extraction effects); it holds at most
//! StockpileCap plus what buildings add (the ..._RESOURCE_STOCKPILE_CAP modifiers of barracks, stables, armories, ...).
//!
//! Not modelled: resources from city-states and trades, and when the game takes
//! the cost (the stockpile must hold it when production is chosen, counting what other cities' units in
//! production will take; it is taken at completion).
use crate::model::*;
use crate::rules::Rules;
use crate::sym::Sym;

/// (resource, amount) a unit needs to be completed or bought; the amount scales with the game speed like
/// production costs (online: half).
pub fn unit_cost(r: &Rules, unit: Sym, speed: Sym) -> Option<(Sym, f64)> {
    let res = r.lookup_sym("Units", "UnitType", unit, "StrategicResource")?;
    if !r.has_table("Units_XP2") {
        return None;
    }
    let amount = (r.lookup_f("Units_XP2", "UnitType", unit, "ResourceCost") * r.game_speed_cost_multiplier(speed)).round();
    (amount > 0.0).then_some((res, amount))
}

fn accumulates(r: &Rules, res: Sym) -> bool {
    r.has_table("Resource_Consumption") && r.lookup_truthy("Resource_Consumption", "ResourceType", res, "Accumulate")
}

const GRANTS: [&str; 3] = ["EFFECT_ADJUST_PLAYER_FREE_RESOURCE_IMPORT_EXTRACTION", "EFFECT_GRANT_PLAYER_FREE_RESOURCE_EXTRACTED", "EFFECT_GRANT_FREE_RESOURCE_EXTRACTED"];

/// The stockpile's gain this turn, by resource; `e` is the engine of the state.
pub fn income(r: &Rules, st: &State, e: &crate::engine::Engine) -> Vec<(Sym, f64)> {
    let mut out: Vec<(Sym, f64)> = vec![];
    let mut add = |out: &mut Vec<(Sym, f64)>, res: Sym, n: f64| match out.iter_mut().find(|x| x.0 == res) {
        Some(x) => x.1 += n,
        None => out.push((res, n)),
    };
    for effect in GRANTS {
        let eff = crate::sym::sym_static(effect);
        let player = e.player_effects(eff);
        let cities = st.cities.iter().flat_map(|c| e.city_effects(eff, c.name));
        for x in player.iter().chain(cities.collect::<Vec<_>>().iter()) {
            if let Some(res) = x.args.get("ResourceType") {
                if accumulates(r, res) {
                    add(&mut out, res, x.args.f("Amount"));
                }
            }
        }
    }
    for c in &st.cities {
        for p in &c.owned {
            let Some(res) = p.resource else { continue };
            if !accumulates(r, res) || r.lookup_sym("Resources", "ResourceType", res, "PrereqTech").map(|t| !st.acc.techs.contains(&t)).unwrap_or(false) {
                continue;
            }
            let improved = p.improvement.map(|i| !p.pillaged && r.list_sym("Improvement_ValidResources", "ImprovementType", i, "ResourceType").contains(&res)).unwrap_or(false);
            let rate = r.lookup_f("Resource_Consumption", "ResourceType", res, if improved { "ImprovedExtractionRate" } else { "BaseExtractionRate" });
            if rate > 0.0 {
                add(&mut out, res, rate);
            }
        }
    }
    out
}

/// What the stockpile of `res` can hold.
pub fn cap(r: &Rules, st: &State, res: Sym) -> f64 {
    let base = r.lookup_f("Resource_Consumption", "ResourceType", res, "StockpileCap");
    let added: f64 = st.cities.iter().flat_map(|c| c.buildings.iter()).map(|b| building_cap(r, *b)).sum();
    base + added
}

/// The stockpile cap a building adds (its ..._RESOURCE_STOCKPILE_CAP modifiers).
fn building_cap(r: &Rules, b: Sym) -> f64 {
    r.list_sym("BuildingModifiers", "BuildingType", b, "ModifierId").into_iter()
        .filter(|m| r.lookup_sym("Modifiers", "ModifierId", *m, "ModifierType").map(|t| t.as_string().ends_with("RESOURCE_STOCKPILE_CAP")).unwrap_or(false))
        .map(|m| {
            let t = r.tab("ModifierArguments");
            t.find("ModifierId", m).iter().find(|&&i| t.v(i, "Name").sym() == Some(crate::s!("Amount"))).map(|&i| t.v(i, "Value").f()).unwrap_or(0.0)
        })
        .sum()
}

/// The stockpile of `res` less what the units in production in the cities other than `except` will take.
pub fn available(r: &Rules, st: &State, res: Sym, except: Option<Sym>, speed: Sym) -> f64 {
    let stock = st.acc.strategic.get(&res).copied().unwrap_or(0.0);
    let reserved: f64 = st.cities.iter().filter(|c| Some(c.name) != except)
        .filter_map(|c| c.current.and_then(|u| unit_cost(r, u, speed))).filter(|x| x.0 == res).map(|x| x.1).sum();
    stock - reserved
}

/// Resources that come from outside the player's empire, which the saves do not show (gifts and trades of team
/// mates and city-states): (resource, amount per turn, from turn), set once from `--strategic-gift RES:N@T`.
pub static GIFTS: std::sync::OnceLock<Vec<(Sym, f64, i64)>> = std::sync::OnceLock::new();

/// The turn's gain, up to the caps.
pub fn accumulate(r: &Rules, st: &mut State, e: &crate::engine::Engine) {
    let mut gains = income(r, st, e);
    for (res, n, from) in GIFTS.get().into_iter().flatten() {
        if st.turn > *from {
            match gains.iter_mut().find(|x| x.0 == *res) {
                Some(x) => x.1 += n,
                None => gains.push((*res, *n)),
            }
        }
    }
    for (res, gain) in gains {
        let cap = cap(r, st, res);
        let s = st.acc.strategic.entry_or(res, 0.0);
        *s = (*s + gain).min(cap.max(*s));
    }
}

/// Takes a completed or bought unit's cost from the stockpile.
pub fn pay(r: &Rules, acc: &mut PlayerAcc, unit: Sym, speed: Sym) {
    if let Some((res, amount)) = unit_cost(r, unit, speed) {
        *acc.strategic.entry_or(res, 0.0) -= amount;
    }
}
