//! Where improvements, districts and wonders may go (civ6sim/placement.py).
use crate::engine::{ss, Engine};
use crate::model::*;
use crate::rules::Rules;
use crate::sym::Sym;
use std::collections::{BTreeSet, HashSet};

fn water(t: Sym) -> bool {
    t == s!("TERRAIN_COAST") || t == s!("TERRAIN_OCEAN")
}
fn mountain(t: Sym) -> bool {
    t.ends_with("_MOUNTAIN")
}

/// One buildable improvement's placement rules, read once.
pub struct ImpInfo {
    imp: Sym,
    builder: bool,
    pt: Option<Sym>,
    pc: Option<Sym>,
    trait_: Option<Sym>,
    resources: Vec<(Sym, bool)>,
    terrains: Vec<(Sym, Option<Sym>, Option<Sym>)>,
    features: Vec<(Sym, Option<Sym>, Option<Sym>)>,
    requires_river: bool,
    same_adjacent_valid: bool,
    coast: bool,
    adjacent_to_land: bool,
    adj_terrain_n: i64,
    adj_terrains: Vec<Sym>,
    adj_bonus_lux: bool,
    adj_lux: bool,
    no_adj_specialty: bool,
    bad_features: Vec<Sym>,
    adj_resources: Vec<Sym>,
    one_per_city: bool,
    min_appeal: f64,
}

fn imp_infos(r: &Rules) -> &Vec<ImpInfo> {
    r.imp_cache.get_or_init(|| {
        let t = r.tab("Improvements");
        let vbu = r.tab("Improvement_ValidBuildUnits");
        let vr = r.tab("Improvement_ValidResources");
        let vt = r.tab("Improvement_ValidTerrains");
        let vf = r.tab("Improvement_ValidFeatures");
        let s = |tab: &crate::rules::Table, i: u32, c: &str| if tab.has_col(c) { tab.v(i, c).sym().filter(|x| !x.is_empty()) } else { None };
        let mut out = vec![];
        for i in r.scan("Improvements", "Improvements") {
            if !t.v(i, "Buildable").truthy() {
                continue;
            }
            let imp = t.v(i, "ImprovementType").sym().unwrap();
            let b = |c: &str| t.has_col(c) && t.v(i, c).truthy();
            out.push(ImpInfo {
                imp,
                builder: vbu.find("ImprovementType", imp).iter().any(|&j| vbu.v(j, "UnitType").sym() == Some(s!("UNIT_BUILDER"))),
                pt: s(t, i, "PrereqTech"),
                pc: s(t, i, "PrereqCivic"),
                trait_: s(t, i, "TraitType"),
                resources: vr.find("ImprovementType", imp).iter().map(|&j| (vr.v(j, "ResourceType").sym().unwrap(), vr.v(j, "MustRemoveFeature").truthy())).collect(),
                terrains: vt.find("ImprovementType", imp).iter().map(|&j| (vt.v(j, "TerrainType").sym().unwrap(), s(vt, j, "PrereqTech"), s(vt, j, "PrereqCivic"))).collect(),
                features: vf.find("ImprovementType", imp).iter().map(|&j| (vf.v(j, "FeatureType").sym().unwrap(), s(vf, j, "PrereqTech"), s(vf, j, "PrereqCivic"))).collect(),
                requires_river: b("RequiresRiver"),
                same_adjacent_valid: b("SameAdjacentValid"),
                coast: b("Coast"),
                adjacent_to_land: b("AdjacentToLand"),
                adj_terrain_n: t.v(i, "ValidAdjacentTerrainAmount").i(),
                adj_terrains: r.list_sym("Improvement_ValidAdjacentTerrains", "ImprovementType", imp, "TerrainType"),
                adj_bonus_lux: b("RequiresAdjacentBonusOrLuxury"),
                adj_lux: b("RequiresAdjacentLuxury"),
                no_adj_specialty: b("NoAdjacentSpecialtyDistrict"),
                bad_features: r.list_sym("Improvement_InvalidAdjacentFeatures", "ImprovementType", imp, "FeatureType"),
                adj_resources: r.list_sym("Improvement_ValidAdjacentResources", "ImprovementType", imp, "ResourceType"),
                one_per_city: b("OnePerCity"),
                min_appeal: t.v(i, "MinimumAppeal").f(),
            });
        }
        out
    })
}

/// Improvements a builder may put on `plot` now. `neighbours`: neighbour plots with districtType set.
pub fn improvement_options(r: &Rules, e: &Engine, plot: &Plot, neighbours: &[Plot], techs: &BTreeSet<Sym>, civics: &BTreeSet<Sym>,
                           city: Option<&City>, is_lake: &dyn Fn(XY) -> bool) -> Vec<Sym> {
    let _g = crate::prof::start(11);
    let mut out = vec![];
    let (terrain, feature, resource) = (plot.terrain, plot.feature, plot.resource);
    let mut allowed: Vec<Sym> = e.player_effects(s!("EFFECT_ADJUST_PLAYER_VALID_IMPROVEMENT")).iter().filter_map(|x| x.args.get("ImprovementType")).collect();
    let mut extra_terrain: Vec<(Sym, Sym)> = vec![];
    if let Some(c) = city {
        allowed.extend(e.city_effects(s!("EFFECT_ADJUST_CITY_ALLOWED_IMPROVEMENT"), c.name).iter().filter_map(|x| x.args.get("ImprovementType")));
        extra_terrain = e.city_effects(s!("EFFECT_ADJUST_IMPROVEMENT_VALID_TERRAIN"), c.name).iter()
            .filter_map(|x| Some((x.args.get("ImprovementType")?, x.args.get("TerrainType")?))).collect();
    }
    let ok_prereq = |a: Option<Sym>, b: Option<Sym>| a.map(|x| techs.contains(&x)).unwrap_or(true) && b.map(|x| civics.contains(&x)).unwrap_or(true);
    let revealed = resource.map(|res| r.resource_revealed(res, techs)).unwrap_or(false);
    for info in imp_infos(r) {
        let imp = info.imp;
        if !info.builder || !ok_prereq(info.pt, info.pc) {
            continue;
        }
        if let Some(tr) = info.trait_ {
            if !r.civ_has_trait(e.env.p.civilization, e.env.p.leader, tr) && !allowed.contains(&imp) {
                continue;
            }
        }
        if revealed {
            let Some(&(_, must_remove)) = info.resources.iter().find(|(res, _)| Some(*res) == resource) else { continue };
            if feature.is_some() && must_remove {
                continue;
            }
        } else {
            let mut ok_t = info.terrains.iter().any(|&(t, a, b)| t == terrain && ok_prereq(a, b)) || extra_terrain.contains(&(imp, terrain));
            if let Some(f) = feature {
                let frs: Vec<&(Sym, Option<Sym>, Option<Sym>)> = info.features.iter().filter(|x| x.0 == f).collect();
                if !frs.iter().any(|&&(_, a, b)| ok_prereq(a, b)) {
                    continue;
                }
                ok_t = ok_t || (!frs.is_empty() && info.terrains.is_empty());
            }
            if !ok_t {
                continue;
            }
        }
        if info.requires_river && !plot.river {
            continue;
        }
        if !info.same_adjacent_valid && neighbours.iter().any(|n| n.improvement == Some(imp)) {
            continue;
        }
        if info.coast && !neighbours.iter().any(|n| water(n.terrain) && !is_lake(n.xy())) {
            continue;
        }
        if info.adjacent_to_land && !neighbours.iter().any(|n| !water(n.terrain)) {
            continue;
        }
        if info.adj_terrain_n != 0 && (neighbours.iter().filter(|n| info.adj_terrains.contains(&n.terrain)).count() as i64) < info.adj_terrain_n {
            continue;
        }
        if info.adj_bonus_lux && !neighbours.iter().any(|n| n.resource.map(|res| {
            let c = r.resource_class(res);
            c == Some(s!("RESOURCECLASS_BONUS")) || c == Some(s!("RESOURCECLASS_LUXURY"))
        }).unwrap_or(false)) {
            continue;
        }
        if info.adj_lux && !neighbours.iter().any(|n| n.resource.map(|res| r.resource_class(res) == Some(s!("RESOURCECLASS_LUXURY"))).unwrap_or(false)) {
            continue;
        }
        if info.no_adj_specialty && neighbours.iter().any(|n| n.district_type.map(|d| r.district_requires_population(d)).unwrap_or(false)) {
            continue;
        }
        if !info.bad_features.is_empty() && neighbours.iter().any(|n| n.feature.map(|f| info.bad_features.contains(&f)).unwrap_or(false)) {
            continue;
        }
        if !info.adj_resources.is_empty() && !neighbours.iter().any(|n| n.resource.map(|x| info.adj_resources.contains(&x)).unwrap_or(false)) {
            continue;
        }
        if info.one_per_city && city.map(|c| c.owned.iter().any(|p| p.improvement == Some(imp))).unwrap_or(false) {
            continue;
        }
        if info.min_appeal != 0.0 {
            let plots: crate::yields::Plots = neighbours.iter().map(|n| (n.xy(), n.clone())).collect();
            let am = city.map(|c| crate::yields::appeal_modifiers(e, c.name));
            if crate::yields::appeal(r, plot, Some(&plots), am.as_ref()) < info.min_appeal {
                continue;
            }
        }
        out.push(imp);
    }
    out
}

pub fn district_ok(r: &Rules, e: &Engine, dist: Sym, plot: &Plot, neighbours: &[Plot], city: &City, taken: &HashSet<XY>, is_lake: &dyn Fn(XY) -> bool) -> bool {
    let xy = plot.xy();
    if taken.contains(&xy) || plot.improvement.is_some() {
        return false;
    }
    let t = r.tab("Districts");
    let Some(i) = t.first("DistrictType", dist) else { return false };
    let b = |c: &str| t.v(i, c).truthy();
    let (coast, no_adj_city, aqueduct) = (b("Coast"), b("NoAdjacentCity"), b("Aqueduct"));
    let center = city.center;
    if let Some(res) = plot.resource {
        let c = r.resource_class(res);
        if (c == Some(s!("RESOURCECLASS_LUXURY")) || c == Some(s!("RESOURCECLASS_STRATEGIC"))) && r.resource_revealed(res, &e.env.techs) {
            return false;
        }
    }
    if let Some(f) = plot.feature {
        if r.is_natural_wonder(f) || r.lookup_truthy("Features", "FeatureType", f, "Impassable") {
            return false;
        }
    }
    if mountain(plot.terrain) {
        return false;
    }
    let valid_t = r.list_sym("District_ValidTerrains", "DistrictType", dist, "TerrainType");
    if coast {
        if !water(plot.terrain) || !neighbours.iter().any(|n| !water(n.terrain)) {
            return false;
        }
    } else if water(plot.terrain) {
        return false;
    }
    if !valid_t.is_empty() && !valid_t.contains(&plot.terrain) {
        return false;
    }
    let required_f = r.list_sym("District_RequiredFeatures", "DistrictType", dist, "FeatureType");
    if !required_f.is_empty() && !plot.feature.map(|f| required_f.contains(&f)).unwrap_or(false) {
        return false;
    }
    let near_center = neighbours.iter().any(|n| n.xy() == center);
    if no_adj_city && near_center {
        return false;
    }
    if aqueduct {
        if !near_center {
            return false;
        }
        let fresh = plot.river || neighbours.iter().any(|n| is_lake(n.xy()) || mountain(n.terrain)
            || n.feature.map(|f| r.lookup_truthy("Features", "FeatureType", f, "AddsFreshWater")).unwrap_or(false));
        if !fresh {
            return false;
        }
    }
    let specialty = r.district_requires_population(dist);
    let need_f: Vec<Sym> = e.player_effects(s!("EFFECT_ADJUST_PLAYER_FEAUTE_REQUIRED_FOR_SPECIALTY_DISTRICTS")).iter().filter_map(|x| x.args.get("FeatureType")).collect();
    if !need_f.is_empty() && specialty && !coast && !plot.feature.map(|f| need_f.contains(&f)).unwrap_or(false) {
        return false;
    }
    if specialty && !e.player_effects(s!("EFFECT_ADJUST_PLAYER_SPECIALTY_DISTRICT_CANNOT_BE_BUILT_ADJACENT_TO_CITY")).is_empty() && near_center {
        return false;
    }
    if let Some(f) = plot.feature {
        if need_f.is_empty() {
            let removable = r.lookup_truthy("Features", "FeatureType", f, "Removable");
            let valid = r.has_table("Features_XP2") && r.lookup_truthy("Features_XP2", "FeatureType", f, "ValidDistrictPlacement");
            if !removable && !valid {
                return false;
            }
        }
    }
    true
}

pub fn wonder_ok(r: &Rules, e: &Engine, wonder: Sym, plot: &Plot, neighbours: &[Plot], city: &City, taken: &HashSet<XY>, is_lake: &dyn Fn(XY) -> bool) -> bool {
    let xy = plot.xy();
    if taken.contains(&xy) || plot.improvement.is_some() {
        return false;
    }
    let t = r.tab("Buildings");
    let Some(i) = t.first("BuildingType", wonder) else { return false };
    let s = |c: &str| t.v(i, c).sym().filter(|x| !x.is_empty());
    let b = |c: &str| t.v(i, c).truthy();
    let (adj_d, adj_res, adj_imp) = (s("AdjacentDistrict"), s("AdjacentResource"), s("AdjacentImprovement"));
    let (river, coast, lake_only, no_lake, adj_cap, adj_mtn, adj_river, adj_land) = (b("RequiresRiver"), b("Coast"), b("MustBeLake"),
        b("MustNotBeLake"), b("AdjacentCapital"), b("AdjacentToMountain"), b("RequiresAdjacentRiver"), b("MustBeAdjacentLand"));
    let w = water(plot.terrain);
    if coast || lake_only {
        if !w {
            return false;
        }
        if lake_only && !is_lake(xy) {
            return false;
        }
        if no_lake && is_lake(xy) {
            return false;
        }
        if adj_land && !neighbours.iter().any(|n| !water(n.terrain)) {
            return false;
        }
    } else if w {
        return false;
    }
    if mountain(plot.terrain) {
        return false;
    }
    let valid_t = r.list_sym("Building_ValidTerrains", "BuildingType", wonder, "TerrainType");
    if !valid_t.is_empty() && !valid_t.contains(&plot.terrain) {
        return false;
    }
    let keep_f = r.list_sym("Building_ValidFeatures", "BuildingType", wonder, "FeatureType");
    let need_f = r.list_sym("Building_RequiredFeatures", "BuildingType", wonder, "FeatureType");
    if !need_f.is_empty() && !plot.feature.map(|f| need_f.contains(&f)).unwrap_or(false) {
        return false;
    }
    if let Some(f) = plot.feature {
        if !keep_f.contains(&f) && !need_f.contains(&f) {
            let removable = r.lookup_truthy("Features", "FeatureType", f, "Removable");
            let valid = r.has_table("Features_XP2") && r.lookup_truthy("Features_XP2", "FeatureType", f, "ValidWonderPlacement");
            if !removable && !valid {
                return false;
            }
        }
    }
    if let Some(res) = plot.resource {
        let c = r.resource_class(res);
        if (c == Some(s!("RESOURCECLASS_LUXURY")) || c == Some(s!("RESOURCECLASS_STRATEGIC"))) && r.resource_revealed(res, &e.env.techs) {
            return false;
        }
    }
    if river && !plot.river {
        return false;
    }
    if adj_river && !neighbours.iter().any(|n| n.river) && !plot.river {
        return false;
    }
    if let Some(d) = adj_d {
        if !neighbours.iter().any(|n| n.district_type.map(|t| r.district_is(t, d)).unwrap_or(false)) {
            return false;
        }
    }
    if let Some(res) = adj_res {
        if !neighbours.iter().any(|n| n.resource == Some(res)) {
            return false;
        }
    }
    if let Some(imp) = adj_imp {
        if !neighbours.iter().any(|n| n.improvement == Some(imp)) {
            return false;
        }
    }
    if adj_cap && !neighbours.iter().any(|n| n.xy() == city.center && city.capital) {
        return false;
    }
    if adj_mtn && !neighbours.iter().any(|n| mountain(n.terrain)) {
        return false;
    }
    true
}
