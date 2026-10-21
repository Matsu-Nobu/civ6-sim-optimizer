//! Yields (civ6sim/yields.py): tile yields, appeal, adjacency, modifier yields, trade routes, city totals.
use crate::engine::{ss, Engine, PlotMod};
use crate::geo::{hex_distance, neighbour_xy};
use crate::model::*;
use crate::rules::{yidx, ysyms, Args, Has, Req, Rules, Y};
use crate::sym::Sym;
use std::collections::{BTreeSet, HashMap};

pub type Plots = HashMap<XY, Plot>;

pub struct Ctx<'a> {
    pub techs: &'a BTreeSet<Sym>,
    pub civics: &'a BTreeSet<Sym>,
    pub plots: Option<&'a Plots>,
    pub city: Option<&'a City>,
    pub appeal_mods: Option<&'a AppealMods>,
}

#[derive(Clone, Debug, Default)]
pub struct AppealMods {
    pub city: f64,
    pub feature: Vec<(Sym, f64)>,
    pub near_unimproved_feature: f64,
}

pub fn add(y: &mut Y, k: Sym, v: f64) {
    if let Some(i) = yidx(k) {
        y[i] += v;
    }
}

// ---------------------------------------------------------------- plots
/// (x, y) -> plot for every owned plot and the plots bordering them, with districtType filled in.
pub fn plots_by_xy_of(cities: &[City]) -> Plots {
    let mut kinds: HashMap<XY, Sym> = HashMap::new();
    let mut done: HashMap<XY, bool> = HashMap::new();
    for c in cities {
        for d in &c.districts {
            kinds.insert((d.x, d.y), d.dtype);
            done.insert((d.x, d.y), d.complete);
        }
    }
    let mut out = Plots::new();
    for c in cities {
        for p in &c.near {
            out.insert(p.xy(), p.clone());
        }
    }
    for c in cities {
        for p in &c.owned {
            let mut q = p.clone();
            q.district_type = kinds.get(&p.xy()).copied();
            q.district_complete = done.get(&p.xy()).copied();
            q.wonder_complete = p.wonder.map(|w| c.buildings.contains(&w));
            out.insert(p.xy(), q);
        }
    }
    out
}

fn plots_in_range<'a>(p: &Plot, ctx: &'a Ctx, lo: i32, hi: i32) -> Vec<&'a Plot> {
    let Some(plots) = ctx.plots else { return vec![] };
    if hi == 1 && lo <= 1 {
        return neighbour_xy(p.x, p.y).iter().filter_map(|xy| plots.get(xy)).collect();
    }
    plots.iter().filter(|(k, _)| { let d = hex_distance(p.xy(), **k); lo <= d && d <= hi }).map(|(_, q)| q).collect()
}

fn rng(a: &Args) -> (i32, i32) {
    let g = |n: &'static str, d: i32| a.get(n).filter(|x| !x.is_empty()).map(|x| x.with(|s| s.parse::<i32>().unwrap_or(d))).unwrap_or(d);
    (g("MinRange", 1), g("MaxRange", 1))
}

fn improved_class(r: &Rules, p: &Plot, cls: Option<Sym>) -> bool {
    match p.resource {
        Some(res) => r.resource_class(res) == cls && !p.pillaged && p.improvement.map(|i| r.valid_improvements(res).contains(&i)).unwrap_or(false),
        None => false,
    }
}

pub fn is_impassable(r: &Rules, terrain: Option<Sym>, feature: Option<Sym>) -> bool {
    let t = terrain.map(|t| r.lookup_truthy("Terrains", "TerrainType", t, "Impassable")).unwrap_or(false);
    let f = feature.map(|f| r.lookup_truthy("Features", "FeatureType", f, "Impassable")).unwrap_or(false);
    t || f
}

const PLOT_REQS: [&str; 30] = [
    "REQUIREMENT_PLOT_RESOURCE_TAG_MATCHES", "REQUIREMENT_PLOT_RESOURCE_TYPE_MATCHES", "REQUIREMENT_PLOT_RESOURCE_CLASS_TYPE_MATCHES",
    "REQUIREMENT_PLOT_IMPROVED_RESOURCE_CLASS_TYPE_MATCHES", "REQUIREMENT_PLOT_RESOURCE_VISIBLE", "REQUIREMENT_PLAYER_HAS_RESOURCE_VISIBILITY",
    "REQUIREMENT_PLOT_IMPROVEMENT_TYPE_MATCHES", "REQUIREMENT_PLOT_FEATURE_TYPE_MATCHES", "REQUIREMENT_PLOT_TERRAIN_TYPE_MATCHES",
    "REQUIREMENT_PLOT_TERRAIN_CLASS_MATCHES", "REQUIREMENT_PLOT_IS_HILLS", "REQUIREMENT_PLOT_IS_MOUNTAIN", "REQUIREMENT_PLOT_IS_IMPASSABLE",
    "REQUIREMENT_PLOT_HAS_ANY_FEATURE", "REQUIREMENT_PLOT_HAS_ANY_RESOURCE", "REQUIREMENT_PLOT_HAS_ANY_IMPROVEMENT",
    "REQUIREMENT_PLOT_DISTRICT_TYPE_MATCHES", "REQUIREMENT_PLOT_ADJACENT_TO_RIVER", "REQUIREMENT_PLOT_IS_LAKE",
    "REQUIREMENT_PLOT_ADJACENT_IMPROVEMENT_TYPE_MATCHES", "REQUIREMENT_PLOT_ADJACENT_DISTRICT_TYPE_MATCHES",
    "REQUIREMENT_PLOT_ADJACENT_FEATURE_TYPE_MATCHES", "REQUIREMENT_PLAYER_HAS_TECHNOLOGY", "REQUIREMENT_PLAYER_HAS_CIVIC",
    "REQUIREMENT_CITY_HAS_BUILDING", "REQUIREMENT_CITY_HAS_DISTRICT", "REQUIREMENT_CITY_FOLLOWS_PANTHEON",
    "REQUIREMENT_PLOT_IS_APPEAL_BETWEEN", "REQUIREMENT_REQUIREMENTSET_IS_MET", "REQUIREMENT_PLOT_AT_XY",
];

pub fn is_plot_requirement(rt: Sym) -> bool {
    thread_local!(static SET: std::collections::HashSet<Sym> = PLOT_REQS.iter().map(|n| ss(n)).collect());
    SET.with(|s| s.contains(&rt))
}

/// PLOT_REQUIREMENTS[rtype](rules, plot, args, ctx)
pub fn plot_requirement(r: &Rules, rt: Sym, p: &Plot, a: &Args, c: &Ctx) -> bool {
    if (rt == s!("REQUIREMENT_PLOT_RESOURCE_TAG_MATCHES")) {
        a.get("Tag").map(|t| r.tags_of(p.resource).contains(&t)).unwrap_or(false)
    } else if (rt == s!("REQUIREMENT_PLOT_RESOURCE_TYPE_MATCHES")) {
        p.resource == a.get("ResourceType")
    } else if (rt == s!("REQUIREMENT_PLOT_RESOURCE_CLASS_TYPE_MATCHES")) {
        p.resource.is_some() && r.resource_class(p.resource.unwrap()) == a.get("ResourceClassType")
    } else if (rt == s!("REQUIREMENT_PLOT_IMPROVED_RESOURCE_CLASS_TYPE_MATCHES")) {
        improved_class(r, p, a.get("ResourceClassType"))
    } else if (rt == s!("REQUIREMENT_PLOT_RESOURCE_VISIBLE")) {
        p.resource.map(|res| r.resource_revealed(res, c.techs)).unwrap_or(false)
    } else if (rt == s!("REQUIREMENT_PLAYER_HAS_RESOURCE_VISIBILITY")) {
        a.get("ResourceType").map(|res| r.resource_revealed(res, c.techs)).unwrap_or(true)
    } else if (rt == s!("REQUIREMENT_PLOT_IMPROVEMENT_TYPE_MATCHES")) {
        p.improvement == a.get("ImprovementType")
    } else if (rt == s!("REQUIREMENT_PLOT_FEATURE_TYPE_MATCHES")) {
        p.feature == a.get("FeatureType")
    } else if (rt == s!("REQUIREMENT_PLOT_TERRAIN_TYPE_MATCHES")) {
        Some(p.terrain) == a.get("TerrainType")
    } else if (rt == s!("REQUIREMENT_PLOT_TERRAIN_CLASS_MATCHES")) {
        let t = r.tab("TerrainClass_Terrains");
        a.get("TerrainClass").map(|cls| t.find("TerrainClassType", cls).iter().any(|&i| t.v(i, "TerrainType").sym() == Some(p.terrain))).unwrap_or(false)
    } else if (rt == s!("REQUIREMENT_PLOT_IS_HILLS")) {
        p.terrain.ends_with("_HILLS")
    } else if (rt == s!("REQUIREMENT_PLOT_IS_MOUNTAIN")) {
        p.terrain.ends_with("_MOUNTAIN")
    } else if (rt == s!("REQUIREMENT_PLOT_IS_IMPASSABLE")) {
        is_impassable(r, Some(p.terrain), p.feature)
    } else if (rt == s!("REQUIREMENT_PLOT_HAS_ANY_FEATURE")) {
        p.feature.is_some()
    } else if (rt == s!("REQUIREMENT_PLOT_HAS_ANY_RESOURCE")) {
        p.resource.is_some()
    } else if (rt == s!("REQUIREMENT_PLOT_HAS_ANY_IMPROVEMENT")) {
        p.improvement.is_some()
    } else if (rt == s!("REQUIREMENT_PLOT_DISTRICT_TYPE_MATCHES")) {
        p.district_type == a.get("DistrictType")
    } else if (rt == s!("REQUIREMENT_PLOT_ADJACENT_TO_RIVER")) {
        p.river
    } else if (rt == s!("REQUIREMENT_PLOT_IS_LAKE")) {
        p.lake
    } else if (rt == s!("REQUIREMENT_PLOT_ADJACENT_IMPROVEMENT_TYPE_MATCHES")) {
        let (lo, hi) = rng(a);
        let want = a.get("ImprovementType");
        plots_in_range(p, c, lo, hi).iter().any(|q| q.improvement == want && !q.pillaged)
    } else if (rt == s!("REQUIREMENT_PLOT_ADJACENT_DISTRICT_TYPE_MATCHES")) {
        let (lo, hi) = rng(a);
        let want = a.get("DistrictType");
        plots_in_range(p, c, lo, hi).iter().any(|q| q.district_type == want)
    } else if (rt == s!("REQUIREMENT_PLOT_ADJACENT_FEATURE_TYPE_MATCHES")) {
        let want = a.get("FeatureType");
        plots_in_range(p, c, 1, 1).iter().any(|q| q.feature == want)
    } else if (rt == s!("REQUIREMENT_PLAYER_HAS_TECHNOLOGY")) {
        a.get("TechnologyType").map(|t| c.techs.contains(&t)).unwrap_or(false)
    } else if (rt == s!("REQUIREMENT_PLAYER_HAS_CIVIC")) {
        a.get("CivicType").map(|t| c.civics.contains(&t)).unwrap_or(false)
    } else if (rt == s!("REQUIREMENT_CITY_HAS_BUILDING")) {
        c.city.map(|ci| a.get("BuildingType").map(|b| ci.buildings.contains(&b)).unwrap_or(false)).unwrap_or(false)
    } else if (rt == s!("REQUIREMENT_CITY_HAS_DISTRICT")) {
        c.city.map(|ci| a.get("DistrictType").map(|d| ci.has_complete_district(d)).unwrap_or(false)).unwrap_or(false)
    } else if (rt == s!("REQUIREMENT_CITY_FOLLOWS_PANTHEON")) {
        true
    } else if (rt == s!("REQUIREMENT_PLOT_IS_APPEAL_BETWEEN")) {
        let lo = a.get("MinimumAppeal").filter(|x| !x.is_empty()).map(|x| x.with(|s| s.parse::<f64>().unwrap_or(-999.0))).unwrap_or(-999.0);
        let hi = a.get("MaximumAppeal").filter(|x| !x.is_empty()).map(|x| x.with(|s| s.parse::<f64>().unwrap_or(999.0))).unwrap_or(999.0);
        let ap = appeal(r, p, c.plots, c.appeal_mods);
        lo <= ap && ap <= hi
    } else if (rt == s!("REQUIREMENT_REQUIREMENTSET_IS_MET")) {
        let Some(rs) = a.get("RequirementSetId") else { return false };
        let set = r.requirement_set(rs);
        let mut results = vec![];
        for q in &set.0 {
            if !is_plot_requirement(q.rtype) {
                return false;
            }
            results.push(plot_requirement(r, q.rtype, p, &q.args, c) != q.inverse);
        }
        if set.1 { results.iter().any(|&x| x) } else { results.iter().all(|&x| x) }
    } else if (rt == s!("REQUIREMENT_PLOT_AT_XY")) {
        a.get("X").map(|x| x.with(|s| s.parse::<i32>().unwrap_or(-9999))) == Some(p.x)
            && a.get("Y").map(|y| y.with(|s| s.parse::<i32>().unwrap_or(-9999))) == Some(p.y)
    } else {
        false
    }
}

pub fn is_city_requirement(rt: Sym) -> bool {
    thread_local!(static SET: std::collections::HashSet<Sym> = ["REQUIREMENT_CITY_HAS_DISTRICT", "REQUIREMENT_CITY_HAS_BUILDING",
        "REQUIREMENT_CITY_HAS_X_SPECIALTY_DISTRICTS", "REQUIREMENT_CITY_HAS_HIGH_LOYALTY"].iter().map(|n| ss(n)).collect());
    SET.with(|s| s.contains(&rt))
}

pub fn city_requirement(r: &Rules, rt: Sym, c: &City, a: &Args) -> bool {
    if (rt == s!("REQUIREMENT_CITY_HAS_DISTRICT")) {
        a.get("DistrictType").map(|d| c.has_complete_district(d)).unwrap_or(false)
    } else if (rt == s!("REQUIREMENT_CITY_HAS_BUILDING")) {
        a.get("BuildingType").map(|b| r.city_has_building(&c.buildings, b)).unwrap_or(false)
    } else if (rt == s!("REQUIREMENT_CITY_HAS_X_SPECIALTY_DISTRICTS")) {
        specialty_district_count(r, c) as f64 >= a.f("Amount")
    } else {
        true
    }
}

pub fn specialty_district_count(r: &Rules, c: &City) -> usize {
    c.districts.iter().filter(|d| d.complete && r.district_requires_population(d.dtype)).count()
}

fn appeal_of(r: &Rules, table: &str, col: &str, key: Option<Sym>) -> f64 {
    match key {
        Some(k) => r.lookup(table, col, k, "Appeal").map(|v| v.f()).unwrap_or(0.0),
        None => 0.0,
    }
}

pub fn appeal(r: &Rules, p: &Plot, plots: Option<&Plots>, mods: Option<&AppealMods>) -> f64 {
    if p.terrain == s!("TERRAIN_COAST") || p.terrain == s!("TERRAIN_OCEAN") {
        return 0.0;
    }
    let def = AppealMods::default();
    let mods = mods.unwrap_or(&def);
    let mut total = (if p.river { 1.0 } else { 0.0 }) + mods.city;
    let mut unimproved_feature_near = false;
    for xy in neighbour_xy(p.x, p.y) {
        let Some(q) = plots.and_then(|m| m.get(&xy)) else { continue };
        total += appeal_of(r, "Terrains", "TerrainType", Some(q.terrain));
        total += appeal_of(r, "Features", "FeatureType", q.feature)
            + q.feature.and_then(|f| mods.feature.iter().find(|(k, _)| *k == f).map(|(_, v)| *v)).unwrap_or(0.0);
        if q.feature.is_some() && q.improvement.is_none() && !r.is_natural_wonder(q.feature.unwrap()) {
            unimproved_feature_near = true;
        }
        if !q.pillaged {
            total += appeal_of(r, "Improvements", "ImprovementType", q.improvement);
        }
        if q.wonder_complete != Some(false) {
            total += appeal_of(r, "Districts", "DistrictType", q.district_type);
        }
    }
    if unimproved_feature_near {
        total += mods.near_unimproved_feature;
    }
    total
}

pub fn appeal_modifiers(e: &Engine, city: Sym) -> AppealMods {
    let mut m = AppealMods::default();
    for x in e.city_effects(s!("EFFECT_ADJUST_CITY_APPEAL"), city) {
        m.city += x.args.f("Amount");
    }
    for x in e.city_effects(s!("EFFECT_ADJUST_FEATURE_APPEAL_MODIFIER"), city) {
        let f = x.args.get("FeatureType").unwrap_or(Sym(0));
        if let Some(v) = m.feature.iter_mut().find(|(k, _)| *k == f) {
            v.1 += x.args.f("Amount");
        } else {
            m.feature.push((f, x.args.f("Amount")));
        }
    }
    for x in e.city_effects(s!("EFFECT_ADJUST_FEATURE_NO_IMPROVEMENT_APPEAL_GOVERNOR"), city) {
        m.near_unimproved_feature += x.args.f("Amount");
    }
    m
}

pub fn plot_modifier_yields(r: &Rules, p: &Plot, mods: &[PlotMod], c: &Ctx) -> Vec<(Sym, f64)> {
    let mut out: Vec<(Sym, f64)> = vec![];
    for m in mods {
        let mut results = vec![];
        for q in &m.reqs {
            results.push(plot_requirement(r, q.rtype, p, &q.args, c) != q.inverse);
        }
        if let Some(xy) = m.at {
            results.push(p.xy() == xy);
        }
        let ok = if m.test_any { results.iter().any(|&x| x) } else { results.iter().all(|&x| x) };
        if ok {
            push_y(&mut out, m.yt, m.amount);
        }
    }
    out
}

/// Ordered yield accumulation (a Python defaultdict(float)).
pub fn push_y(out: &mut Vec<(Sym, f64)>, k: Sym, v: f64) {
    if let Some(x) = out.iter_mut().find(|(a, _)| *a == k) {
        x.1 += v;
    } else {
        out.push((k, v));
    }
}

/// Improvement plot modifiers (rules._improvement_plot_modifiers via plot_yield_modifier).
pub fn improvement_plot_modifiers(r: &Rules, imp: Sym) -> Vec<PlotMod> {
    let mut out = vec![];
    for mid in r.list_sym("ImprovementModifiers", "ImprovementType", imp, "ModifierId") {
        out.extend(plot_yield_modifier(r, mid));
    }
    out
}

fn owner_requirements_met(r: &Rules, rs: Sym) -> bool {
    // the player is None here (rules.plot_yield_modifier called without a player): any requirement fails
    let set = r.requirement_set(rs);
    if set.0.is_empty() {
        return !set.1; // all([]) is True, any([]) is False
    }
    false
}

pub fn plot_yield_modifier(r: &Rules, mid: Sym) -> Vec<PlotMod> {
    let Some(m) = r.modifier(mid) else { return vec![] };
    if let Some(o) = m.owner_rs {
        if !owner_requirements_met(r, o) {
            return vec![];
        }
    }
    if let Some(inner) = m.args.get("ModifierId") {
        if m.mtype.ends_with("ATTACH_MODIFIER") {
            return plot_yield_modifier(r, inner);
        }
    }
    let ok_types = ["MODIFIER_CITY_PLOT_YIELDS_ADJUST_PLOT_YIELD", "MODIFIER_PLAYER_ADJUST_PLOT_YIELD", "MODIFIER_SINGLE_PLOT_ADJUST_PLOT_YIELDS"];
    if !ok_types.iter().any(|t| ss(t) == m.mtype) || m.args.get("YieldType").is_none() {
        return vec![];
    }
    let mut reqs: Vec<Req> = vec![];
    let mut test_any = false;
    if let Some(s) = m.subj_rs {
        let set = r.requirement_set(s);
        test_any = set.1;
        for q in &set.0 {
            if !is_plot_requirement(q.rtype) {
                return vec![];
            }
            reqs.push(q.clone());
        }
    }
    vec![PlotMod { yt: m.args.get("YieldType").unwrap(), amount: m.args.f("Amount"), reqs, test_any, at: None }]
}

#[derive(Clone, Debug)]
pub struct AdjRow {
    pub id: Option<Sym>,
    pub yt: Sym,
    pub change: f64,
    pub tiles_required: i64,
    pub other_district: bool,
    pub sea_resource: bool,
    pub terrain: Option<Sym>,
    pub feature: Option<Sym>,
    pub river: bool,
    pub wonder: bool,
    pub natural_wonder: bool,
    pub improvement: Option<Sym>,
    pub district: Option<Sym>,
    pub prereq_civic: Option<Sym>,
    pub prereq_tech: Option<Sym>,
    pub obsolete_civic: Option<Sym>,
    pub obsolete_tech: Option<Sym>,
    pub resource: bool,
    pub resource_class: Option<Sym>,
    pub self_: bool,
}

impl AdjRow {
    pub fn empty(id: Sym, yt: Sym, change: f64, tiles: i64) -> AdjRow {
        AdjRow { id: Some(id), yt, change, tiles_required: tiles, other_district: false, sea_resource: false, terrain: None,
            feature: None, river: false, wonder: false, natural_wonder: false, improvement: None, district: None, prereq_civic: None,
            prereq_tech: None, obsolete_civic: None, obsolete_tech: None, resource: false, resource_class: None, self_: false }
    }
}

fn adj_row(r: &Rules, i: u32) -> AdjRow {
    let t = r.tab("Adjacency_YieldChanges");
    let s = |c: &str| if t.has_col(c) { t.v(i, c).sym().filter(|x| !x.is_empty()) } else { None };
    let b = |c: &str| t.has_col(c) && t.v(i, c).truthy();
    AdjRow {
        id: s("ID"),
        yt: t.v(i, "YieldType").sym().unwrap(),
        change: t.v(i, "YieldChange").f(),
        tiles_required: t.v(i, "TilesRequired").i(),
        other_district: b("OtherDistrictAdjacent"),
        sea_resource: b("AdjacentSeaResource"),
        terrain: s("AdjacentTerrain"),
        feature: s("AdjacentFeature"),
        river: b("AdjacentRiver"),
        wonder: b("AdjacentWonder"),
        natural_wonder: b("AdjacentNaturalWonder"),
        improvement: s("AdjacentImprovement"),
        district: s("AdjacentDistrict"),
        prereq_civic: s("PrereqCivic"),
        prereq_tech: s("PrereqTech"),
        obsolete_civic: s("ObsoleteCivic"),
        obsolete_tech: s("ObsoleteTech"),
        resource: b("AdjacentResource"),
        resource_class: s("AdjacentResourceClass"),
        self_: b("Self"),
    }
}

pub fn district_adjacency_rows(r: &Rules, d: Sym) -> Vec<AdjRow> {
    let t = r.tab("District_Adjacencies");
    let a = r.tab("Adjacency_YieldChanges");
    let mut out = vec![];
    for &i in t.find("DistrictType", d) {
        let id = t.v(i, "YieldChangeId").sym().unwrap();
        for &j in a.find("ID", id) {
            out.push(adj_row(r, j));
        }
    }
    out
}

pub fn improvement_adjacency_rows(r: &Rules, imp: Sym) -> Vec<AdjRow> {
    let t = r.tab("Improvement_Adjacencies");
    let a = r.tab("Adjacency_YieldChanges");
    let mut out = vec![];
    for &i in t.find("ImprovementType", imp) {
        let id = t.v(i, "YieldChangeId").sym().unwrap();
        for &j in a.find("ID", id) {
            out.push(adj_row(r, j));
        }
    }
    out
}

fn has_opt(s: &BTreeSet<Sym>, v: Option<Sym>) -> bool {
    v.map(|x| s.contains(&x)).unwrap_or(false)
}

fn row_live(row: &AdjRow, techs: &BTreeSet<Sym>, civics: &BTreeSet<Sym>) -> bool {
    if (row.prereq_tech.is_some() && !has_opt(techs, row.prereq_tech)) || (row.prereq_civic.is_some() && !has_opt(civics, row.prereq_civic)) {
        return false;
    }
    if has_opt(techs, row.obsolete_tech) || has_opt(civics, row.obsolete_civic) {
        return false;
    }
    true
}

pub fn adjacent_match(r: &Rules, row: &AdjRow, n: &Plot) -> bool {
    if row.other_district {
        return n.district_type.is_some() && n.district_type != Some(s!("DISTRICT_WONDER")) && n.district_complete != Some(false);
    }
    if row.district.is_some() {
        return n.district_type == row.district && n.district_complete != Some(false);
    }
    if row.sea_resource {
        return n.resource.is_some() && (n.terrain == s!("TERRAIN_COAST") || n.terrain == s!("TERRAIN_OCEAN"));
    }
    if row.wonder {
        return n.district_type == Some(s!("DISTRICT_WONDER")) && n.wonder_complete != Some(false);
    }
    if row.terrain.is_some() {
        return Some(n.terrain) == row.terrain;
    }
    if row.feature.is_some() {
        return n.feature == row.feature;
    }
    if row.improvement.is_some() {
        return n.improvement == row.improvement;
    }
    let no_class = row.resource_class.is_none() || row.resource_class == Some(s!("NO_RESOURCECLASS"));
    if row.resource || !no_class {
        return n.resource.is_some() && (no_class || r.resource_class(n.resource.unwrap()) == row.resource_class);
    }
    if row.natural_wonder {
        return n.feature.map(|f| r.is_natural_wonder(f)).unwrap_or(false);
    }
    false
}

pub fn improvement_adjacency(r: &Rules, p: &Plot, plots: Option<&Plots>, techs: &BTreeSet<Sym>, civics: &BTreeSet<Sym>) -> Vec<(Sym, f64)> {
    let mut y = vec![];
    let Some(imp) = p.improvement else { return y };
    let Some(plots) = plots else { return y };
    if p.pillaged || plots.is_empty() {
        return y;
    }
    let neighbours: Vec<&Plot> = neighbour_xy(p.x, p.y).iter().filter_map(|xy| plots.get(xy)).collect();
    for row in improvement_adjacency_rows(r, imp) {
        if !row_live(&row, techs, civics) {
            continue;
        }
        let n = if row.river {
            p.river as i64
        } else {
            neighbours.iter().filter(|q| adjacent_match(r, &row, q) && !(row.improvement.is_some() && q.pillaged)).count() as i64
        };
        push_y(&mut y, row.yt, (n / row.tiles_required.max(1)) as f64 * row.change);
    }
    y
}

#[allow(clippy::too_many_arguments)]
pub fn tile_yields(r: &Rules, plot: &Plot, techs: &BTreeSet<Sym>, civics: &BTreeSet<Sym>, city_center: bool, plot_mods: &[PlotMod],
                   plots: Option<&Plots>, city: Option<&City>, appeal_mods: Option<&AppealMods>) -> Vec<(Sym, f64)> {
    tile_yields_bbcc(r, plot, techs, civics, city_center, false, plot_mods, plots, city, appeal_mods)
}

/// BBCC's minimum city center yields (BBG lobby option; its tables are absent when the game was set up without it).
pub fn bbcc_minimum(r: &Rules, hills: bool) -> Vec<(Sym, f64)> {
    let t = if hills { "Hill_CutOffYieldValues" } else { "Flat_CutOffYieldValues" };
    if !r.has_table(t) {
        return vec![];
    }
    let tab = r.tab(t);
    (0..tab.len() as u32).map(|i| (tab.v(i, "YieldType").sym().unwrap(), tab.v(i, "Amount").f())).collect()
}

#[allow(clippy::too_many_arguments)]
pub fn tile_yields_bbcc(r: &Rules, plot: &Plot, techs: &BTreeSet<Sym>, civics: &BTreeSet<Sym>, city_center: bool, bbcc: bool, plot_mods: &[PlotMod],
                        plots: Option<&Plots>, city: Option<&City>, appeal_mods: Option<&AppealMods>) -> Vec<(Sym, f64)> {
    let mut y: Vec<(Sym, f64)> = vec![];
    let ctx = Ctx { techs, civics, plots, city, appeal_mods };
    if plot.district.map(|d| d != 0).unwrap_or(false) && !city_center {
        return y;
    }
    let mut p = plot.clone();
    if city_center {
        p.district_type = Some(s!("DISTRICT_CITY_CENTER"));
    }
    let resource = p.resource.filter(|&res| r.resource_revealed(res, techs));
    let terrain = if p.feature.is_some() && is_impassable(r, None, p.feature) { None } else { Some(p.terrain) };
    if let Some(t) = terrain {
        // Features.DoubleAdjacentTerrainYield (Torres del Paine): the terrain's own yields count twice next to it
        let mut times = 1.0;
        for xy in neighbour_xy(p.x, p.y) {
            if let Some(f) = plots.and_then(|m| m.get(&xy)).and_then(|q| q.feature) {
                if r.lookup_truthy("Features", "FeatureType", f, "DoubleAdjacentTerrainYield") {
                    times = 2.0;
                }
            }
        }
        for (k, v) in r.yield_rows("Terrain_YieldChanges", "TerrainType", t, "YieldType", "YieldChange") {
            push_y(&mut y, k, times * v);
        }
    }
    if let Some(f) = p.feature {
        for (k, v) in r.yield_rows("Feature_YieldChanges", "FeatureType", f, "YieldType", "YieldChange") {
            push_y(&mut y, k, v);
        }
    }
    if let Some(res) = resource {
        for (k, v) in r.yield_rows("Resource_YieldChanges", "ResourceType", res, "YieldType", "YieldChange") {
            push_y(&mut y, k, v);
        }
    }
    for &(k, v) in &p.yield_changes {
        push_y(&mut y, k, v);
    }
    let imp = p.improvement;
    if let Some(i) = imp {
        if !p.pillaged {
            for (k, v) in r.yield_rows("Improvement_YieldChanges", "ImprovementType", i, "YieldType", "YieldChange") {
                push_y(&mut y, k, v);
            }
            let t = r.tab("Improvement_BonusYieldChanges");
            for row in 0..t.len() as u32 {
                if t.v(row, "ImprovementType").sym() != Some(i) {
                    continue;
                }
                let tech = t.v(row, "PrereqTech").sym().filter(|x| !x.is_empty());
                let civic = t.v(row, "PrereqCivic").sym().filter(|x| !x.is_empty());
                if tech.map(|x| techs.contains(&x)).unwrap_or(true) && civic.map(|x| civics.contains(&x)).unwrap_or(true) {
                    push_y(&mut y, t.v(row, "YieldType").sym().unwrap(), t.v(row, "BonusYieldChange").f());
                }
            }
        }
    }
    if city_center {
        let food = get_y(&y, s!("YIELD_FOOD")).max(r.param("YIELD_FOOD_CITY_TERRAIN_REPLACE"));
        set_y(&mut y, s!("YIELD_FOOD"), food);
        let prod = get_y(&y, s!("YIELD_PRODUCTION")).max(r.param("YIELD_PRODUCTION_CITY_TERRAIN_REPLACE"));
        set_y(&mut y, s!("YIELD_PRODUCTION"), prod);
        if bbcc {
            for (k, v) in bbcc_minimum(r, p.terrain.ends_with("_HILLS")) {
                let cur = get_y(&y, k);
                set_y(&mut y, k, cur.max(v));
            }
        }
    }
    if !is_impassable(r, Some(p.terrain), p.feature) {
        for (k, v) in plot_modifier_yields(r, &p, plot_mods, &ctx) {
            push_y(&mut y, k, v);
        }
        for xy in neighbour_xy(p.x, p.y) {
            if let Some(q) = plots.and_then(|m| m.get(&xy)) {
                if let Some(f) = q.feature {
                    for (k, v) in r.yield_rows("Feature_AdjacentYields", "FeatureType", f, "YieldType", "YieldChange") {
                        push_y(&mut y, k, v);
                    }
                }
            }
        }
    }
    if let Some(i) = imp {
        if !p.pillaged {
            let mods = improvement_plot_modifiers(r, i);
            for (k, v) in plot_modifier_yields(r, &p, &mods, &ctx) {
                push_y(&mut y, k, v);
            }
            for (k, v) in improvement_adjacency(r, &p, plots, techs, civics) {
                push_y(&mut y, k, v);
            }
        }
    }
    y
}

pub fn get_y(y: &[(Sym, f64)], k: Sym) -> f64 {
    y.iter().find(|(a, _)| *a == k).map(|(_, v)| *v).unwrap_or(0.0)
}
fn set_y(y: &mut Vec<(Sym, f64)>, k: Sym, v: f64) {
    if let Some(x) = y.iter_mut().find(|(a, _)| *a == k) {
        x.1 = v;
    } else {
        y.push((k, v));
    }
}

pub fn to_y(v: &[(Sym, f64)]) -> Y {
    let mut y = [0.0; 6];
    for &(k, x) in v {
        add(&mut y, k, x);
    }
    y
}

// ---------------------------------------------------------------- districts
pub fn modifier_adjacency_rows(e: &Engine, city: Sym, district: Sym) -> Vec<AdjRow> {
    let r = e.r;
    let mut rows = vec![];
    for (effect, col) in [("EFFECT_FEATURE_ADJACENCY", 0), ("EFFECT_TERRAIN_ADJACENCY", 1), ("EFFECT_IMPROVEMENT_ADJACENCY", 2)] {
        for x in e.city_effects(crate::sym::sym_static(effect), city) {
            let a = &x.args;
            let Some(dt) = a.get("DistrictType").filter(|d| !d.is_empty()) else { continue };
            if !r.district_is(district, dt) {
                continue;
            }
            let tiles = a.get("TilesRequired").filter(|t| !t.is_empty()).map(|t| t.with(|s| s.parse::<i64>().unwrap_or(1))).unwrap_or(1);
            let mut row = AdjRow::empty(x.mid, a.get("YieldType").unwrap(), a.f("Amount"), tiles);
            match col {
                0 => row.feature = a.get("FeatureType"),
                1 => row.terrain = a.get("TerrainType"),
                _ => row.improvement = a.get("ImprovementType"),
            }
            rows.push(row);
        }
    }
    for (effect, col) in [("EFFECT_DISTRICT_ADJACENCY", 0), ("EFFECT_RIVER_ADJACENCY", 1)] {
        for x in e.city_effects(crate::sym::sym_static(effect), city) {
            let a = &x.args;
            let Some(dt) = a.get("DistrictType").filter(|d| !d.is_empty()) else { continue };
            if !r.district_is(district, dt) {
                continue;
            }
            let mut row = AdjRow::empty(x.mid, a.get("YieldType").unwrap(), a.f("Amount"), 1);
            if col == 0 { row.other_district = true } else { row.river = true }
            rows.push(row);
        }
    }
    rows
}

pub fn excluded_adjacencies(r: &Rules, p: &Player) -> Vec<Sym> {
    let mut out = vec![];
    for &t in r.traits_of(p.civilization, p.leader).iter() {
        for x in r.list_sym("ExcludedAdjacencies", "TraitType", t, "YieldChangeId") {
            if !out.contains(&x) {
                out.push(x);
            }
        }
    }
    out
}

pub fn district_adjacency(r: &Rules, d: &District, techs: &BTreeSet<Sym>, civics: &BTreeSet<Sym>, plots: Option<&Plots>,
                          extra: &[AdjRow], excluded: &[Sym]) -> Vec<(Sym, f64)> {
    let mut y = vec![];
    if !d.complete || d.dtype == s!("DISTRICT_CITY_CENTER") {
        return y;
    }
    let neighbours: Vec<&Plot> = match plots {
        Some(m) if !m.is_empty() => d.neighbours.iter().map(|n| m.get(&n.xy()).unwrap_or(n)).collect(),
        _ => d.neighbours.iter().collect(),
    };
    let base = district_adjacency_rows(r, d.dtype);
    for row in base.iter().chain(extra.iter()) {
        if row.id.map(|i| excluded.contains(&i)).unwrap_or(false) {
            continue;
        }
        if !row_live(row, techs, civics) {
            continue;
        }
        let count = if row.river {
            d.plot.river as i64
        } else if row.self_ {
            continue;
        } else {
            neighbours.iter().filter(|n| adjacent_match(r, row, n)).count() as i64
        };
        push_y(&mut y, row.yt, (count / row.tiles_required.max(1)) as f64 * row.change);
    }
    y
}

// ---------------------------------------------------------------- city yields
pub fn city_plots(c: &City) -> Vec<(Sym, Option<Sym>, Option<Sym>, XY)> {
    let mut seen: Vec<(XY, (Sym, Option<Sym>, Option<Sym>, XY))> = vec![];
    for p in c.worked.iter().chain(c.owned.iter()) {
        let v = (p.terrain, p.resource, p.improvement, p.xy());
        if let Some(x) = seen.iter_mut().find(|(k, _)| *k == p.xy()) {
            x.1 = v;
        } else {
            seen.push((p.xy(), v));
        }
    }
    seen.into_iter().map(|(_, v)| v).collect()
}

fn add_lbl(out: &mut Vec<(Sym, f64)>, k: Sym, v: f64) {
    push_y(out, k, v);
}

/// modifier_city_yields: summed (labels are only for display in Python).
pub fn modifier_city_yields(r: &Rules, e: &Engine, c: &City) -> Vec<(Sym, f64)> {
    let mut out = vec![];
    let n = c.name;
    for x in e.city_effects(s!("EFFECT_ADJUST_CITY_YIELD_CHANGE"), n) {
        add_lbl(&mut out, x.args.get("YieldType").unwrap(), x.args.f("Amount"));
    }
    for x in e.city_effects(s!("EFFECT_ADJUST_CITY_ALL_YIELDS_CHANGE"), n) {
        for &k in ysyms() {
            add_lbl(&mut out, k, x.args.f("Amount"));
        }
    }
    for x in e.city_effects(s!("EFFECT_ADJUST_BUILDING_YIELD_CHANGE"), n) {
        if let Some(b) = x.args.get("BuildingType").filter(|b| !b.is_empty()) {
            if r.city_has_building(&c.buildings, b) {
                add_lbl(&mut out, x.args.get("YieldType").unwrap(), x.args.f("Amount"));
            }
        }
    }
    for x in e.city_effects(s!("EFFECT_ADJUST_CITY_YIELD_PER_POPULATION"), n) {
        add_lbl(&mut out, x.args.get("YieldType").unwrap(), x.args.f("Amount") * c.population as f64);
    }
    for x in e.city_effects(s!("EFFECT_ADJUST_CITY_YIELD_PER_DISTRICT"), n) {
        add_lbl(&mut out, x.args.get("YieldType").unwrap(), x.args.f("Amount") * specialty_district_count(r, c) as f64);
    }
    for effect in ["EFFECT_ADJUST_DISTRICT_YIELD_CHANGE", "EFFECT_ADJUST_DISTRICT_BASE_YIELD_CHANGE"] {
        for (_, x) in e.district_effects(crate::sym::sym_static(effect), n) {
            add_lbl(&mut out, x.args.get("YieldType").unwrap(), x.args.f("Amount"));
        }
    }
    for x in e.city_effects(s!("EFFECT_ADJUST_BUILDING_YIELD_MODIFIER"), n) {
        let bt = x.args.get("BuildingType");
        if let Some(b) = c.buildings.iter().find(|&&b| bt.map(|t| r.building_is(b, t)).unwrap_or(false)) {
            let yt = x.args.get("YieldType").unwrap();
            let base: f64 = r.yield_rows("Building_YieldChanges", "BuildingType", *b, "YieldType", "YieldChange").iter().filter(|(k, _)| *k == yt).map(|(_, v)| v).sum();
            add_lbl(&mut out, yt, base * x.args.f("Amount") / 100.0);
        }
    }
    for x in e.city_effects(s!("EFFECT_ADJUST_BUILDING_YIELD_MODIFIERS_FOR_DISTRICT"), n) {
        let yt = x.args.get("YieldType").unwrap();
        let dt = x.args.get("DistrictType").unwrap_or(Sym(0));
        let mut base = 0.0;
        for &b in &c.buildings {
            if r.building_in_district(b, dt) {
                base += r.yield_rows("Building_YieldChanges", "BuildingType", b, "YieldType", "YieldChange").iter().filter(|(k, _)| *k == yt).map(|(_, v)| v).sum::<f64>();
            }
        }
        if base != 0.0 {
            add_lbl(&mut out, yt, base * x.args.f("Amount") / 100.0);
        }
    }
    for x in e.city_effects(s!("EFFECT_ADJUST_BUILDING_FEATURE_YIELD_CHANGE"), n) {
        let f = x.args.get("FeatureType");
        let mut cnt = 0;
        for d in &c.districts {
            if d.complete && d.plot.feature.is_some() && d.plot.feature == f {
                cnt += c.buildings.iter().filter(|&&b| r.building_in_district(b, d.dtype)).count();
            }
        }
        if cnt > 0 {
            add_lbl(&mut out, x.args.get("YieldType").unwrap(), x.args.f("Amount") * cnt as f64);
        }
    }
    let wonders = c.buildings.iter().filter(|&&b| r.is_wonder(b)).count();
    for x in e.city_effects(s!("EFFECT_ADJUST_WONDER_YIELD_CHANGE"), n).into_iter().chain(e.player_effects(s!("EFFECT_ADJUST_WONDER_YIELD_CHANGE"))) {
        if wonders > 0 {
            add_lbl(&mut out, x.args.get("YieldType").unwrap(), x.args.f("Amount") * wonders as f64);
        }
    }
    let owned = city_plots(c);
    for x in e.city_effects(s!("EFFECT_ADJUST_CITY_YIELD_PER_TERRAIN_TYPE"), n) {
        let t = x.args.get("TerrainType");
        let cnt = owned.iter().filter(|o| Some(o.0) == t).count();
        add_lbl(&mut out, x.args.get("YieldType").unwrap(), x.args.f("Amount") * cnt as f64);
    }
    for x in e.city_effects(s!("EFFECT_ADJUST_YIELD_BY_NUMBER_OF_RESOURCES"), n) {
        let mut classes: Vec<Sym> = vec![];
        for o in &owned {
            if let (Some(res), Some(_)) = (o.1, o.2) {
                if r.resource_revealed(res, &e.env.techs) {
                    if let Some(cl) = r.resource_class(res) {
                        if !classes.contains(&cl) {
                            classes.push(cl);
                        }
                    }
                }
            }
        }
        add_lbl(&mut out, x.args.get("YieldType").unwrap(), x.args.f("Amount") * classes.len() as f64);
    }
    for x in e.city_effects(s!("EFFECT_ADJUST_CITY_YIELD_PER_TERRAIN_CLASS_CITIES_IMPROVEMENT"), n) {
        let imp = x.args.get("ImprovementType");
        if owned.iter().any(|o| o.2.is_some() && o.2 == imp) {
            let mut classes: Vec<Sym> = vec![];
            for o in &owned {
                if let Some(cl) = r.terrain_class(o.0) {
                    if !classes.contains(&cl) {
                        classes.push(cl);
                    }
                }
            }
            add_lbl(&mut out, x.args.get("YieldType").unwrap(), x.args.f("Amount") * classes.len() as f64);
        }
    }
    for x in e.city_effects(s!("EFFECT_ADJUST_CITY_GOLD_FROM_CITIZENS"), n) {
        add_lbl(&mut out, s!("YIELD_GOLD"), x.args.f("Amount") * c.population as f64);
    }
    out
}

pub fn happiness_type(r: &Rules, score: f64) -> Option<Sym> {
    let t = r.tab("Happinesses");
    for i in 0..t.len() as u32 {
        let h = t.v(i, "HappinessType").sym().unwrap();
        if h == s!("HAPPINESS_UNHAPPY") {
            continue;
        }
        let lo = t.v(i, "MinimumAmenityScore");
        let hi = t.v(i, "MaximumAmenityScore");
        if (lo.is_null() || score >= lo.f()) && (hi.is_null() || score <= hi.f()) {
            return Some(h);
        }
    }
    None
}

pub fn faith_conversions(e: &Engine, c: &City, y: &Y) -> Vec<(Sym, f64)> {
    let mut out = vec![];
    for x in e.city_effects(s!("EFFECT_ADJUST_CITY_YIELD_MODIFIER_FROM_FAITH"), c.name) {
        push_y(&mut out, x.args.get("YieldType").unwrap(), y[5] * x.args.f("Amount") / 100.0);
    }
    out
}

pub fn city_yield_percents(r: &Rules, e: &Engine, c: &City, amenity_score: Option<f64>) -> Y {
    let mut out = [0.0; 6];
    for x in e.city_effects(s!("EFFECT_ADJUST_CITY_YIELD_MODIFIER"), c.name) {
        add(&mut out, x.args.get("YieldType").unwrap(), x.args.f("Amount"));
    }
    if c.buildings.iter().any(|&b| r.lookup_truthy("Buildings", "BuildingType", b, "EnabledByReligion")) {
        for x in e.player_effects(s!("EFFECT_ADD_RELIGIOUS_BUILDING_MULTIPLIER")) {
            add(&mut out, x.args.get("YieldType").unwrap(), x.args.f("Multiplier"));
        }
    }
    let titles: usize = e.env.p.governors.iter().flatten().filter(|g| g.established && crate::engine::governor_here(g, c, &e.env.p)).map(|g| g.promotions.len()).sum();
    for x in e.city_effects(s!("EFFECT_ADJUST_CITY_YIELD_MODIFIER_PER_GOVERNOR_TITLE"), c.name) {
        add(&mut out, x.args.get("YieldType").unwrap(), x.args.f("Amount") * titles as f64);
    }
    if let Some(score) = amenity_score {
        let h = happiness_type(r, score);
        for x in e.city_effects(s!("EFFECT_ADJUST_CITY_HAPPINESS_YIELD"), c.name) {
            if x.args.get("HappinessType") == h {
                add(&mut out, x.args.get("YieldType").unwrap(), x.args.f("Amount"));
            }
        }
    }
    out
}

pub fn appeal_district_yields(r: &Rules, c: &City, p: &Plot, plots: &Plots, amods: &AppealMods) -> Vec<(Sym, f64)> {
    let mut y = vec![];
    let near: Vec<&District> = c.districts.iter().filter(|d| d.complete).collect();
    let mut adj: Vec<Sym> = vec![];
    for xy in neighbour_xy(p.x, p.y) {
        if let Some(d) = near.iter().find(|d| (d.x, d.y) == xy) {
            if !adj.contains(&d.dtype) {
                adj.push(d.dtype);
            }
        }
    }
    if adj.is_empty() {
        return y;
    }
    let t = r.tab("Adjacent_AppealYieldChanges");
    let mut ap: Option<f64> = None;
    for i in 0..t.len() as u32 {
        let dist = t.v(i, "DistrictType").sym().unwrap();
        let b = t.v(i, "BuildingType").sym().filter(|x| !x.is_empty());
        if !adj.iter().any(|&a| r.district_is(a, dist)) || b.map(|b| !c.buildings.contains(&b)).unwrap_or(false) {
            continue;
        }
        if t.v(i, "Unimproved").truthy() && p.improvement.is_some() {
            continue;
        }
        let a = *ap.get_or_insert_with(|| appeal(r, p, Some(plots), Some(amods)));
        if t.v(i, "MinimumValue").f() <= a && a <= t.v(i, "MaximumValue").f() {
            push_y(&mut y, t.v(i, "YieldType").sym().unwrap(), t.v(i, "YieldChange").f());
        }
    }
    y
}

pub fn terrain_from_adjacent_improvements(e: &Engine, city: Sym, p: &Plot, plots: &Plots) -> Vec<(Sym, f64)> {
    let mut y = vec![];
    for x in e.city_effects(s!("EFFECT_ADJUST_TERRAIN_YIELD_FROM_ADJACENT_IMPROVEMENTS"), city) {
        if Some(p.terrain) != x.args.get("TerrainType") {
            continue;
        }
        let imp = x.args.get("ImprovementType");
        let cnt = neighbour_xy(p.x, p.y).iter().filter(|xy| plots.get(xy).map(|q| q.improvement.is_some() && q.improvement == imp && !q.pillaged).unwrap_or(false)).count();
        if cnt > 0 {
            push_y(&mut y, x.args.get("YieldType").unwrap(), x.args.f("Amount") * cnt as f64);
        }
    }
    y
}

pub fn specialist_yields(r: &Rules, c: &City, xy: XY) -> Vec<(Sym, f64)> {
    let mut y = vec![];
    let Some(d) = c.districts.iter().find(|d| (d.x, d.y) == xy) else { return y };
    let mut types = vec![d.dtype];
    types.extend(r.list_sym("DistrictReplaces", "CivUniqueDistrictType", d.dtype, "ReplacesDistrictType"));
    for (k, v) in r.yield_rows("District_CitizenYieldChanges", "DistrictType", d.dtype, "YieldType", "YieldChange") {
        push_y(&mut y, k, v);
    }
    for &b in &c.buildings {
        if let Some(pd) = r.lookup_sym("Buildings", "BuildingType", b, "PrereqDistrict") {
            if types.contains(&pd) {
                for (k, v) in r.yield_rows("Building_CitizenYieldChanges", "BuildingType", b, "YieldType", "YieldChange") {
                    push_y(&mut y, k, v);
                }
            }
        }
    }
    y
}

// ---------------------------------------------------------------- trade routes
fn is_city_state(r: &Rules, civ: Option<Sym>) -> bool {
    civ.and_then(|c| r.lookup_sym("Civilizations", "CivilizationType", c, "StartingCivilizationLevelType")) == Some(s!("CIVILIZATION_LEVEL_CITY_STATE"))
}

fn resource_count(r: &Rules, plots: &[(Sym, Option<Sym>, Option<Sym>, Option<XY>)], cls: &str, techs: &BTreeSet<Sym>, center: Option<XY>, radius: Option<i32>) -> f64 {
    let mut n = 0.0;
    for (_, res, _imp, xy) in plots {
        let Some(res) = res else { continue };
        if r.resource_class(*res) != Some(ss(cls)) || !r.resource_revealed(*res, techs) {
            continue;
        }
        if let (Some(rad), Some(xy), Some(c)) = (radius, xy, center) {
            if hex_distance(*xy, c) > rad {
                continue;
            }
        }
        n += 1.0;
    }
    n
}

fn split_args(a: &Args, key: &'static str, amount: &'static str) -> Vec<(Sym, f64)> {
    let ks: Vec<String> = a.get(key).map(|s| s.as_string()).unwrap_or_default().split(',').map(|x| x.trim().to_string()).collect();
    let vs: Vec<String> = a.get(amount).map(|s| s.as_string()).unwrap_or_default().split(',').map(|x| x.trim().to_string()).collect();
    let vs: Vec<String> = if vs.len() == 1 { vec![vs[0].clone(); ks.len()] } else { vs };
    ks.iter().zip(vs.iter()).map(|(k, v)| (crate::sym::sym(k), v.parse::<f64>().unwrap_or(0.0))).collect()
}

fn is_true(a: &Args, k: &'static str) -> bool {
    a.is(k, &["1", "true"])
}

fn route_kind_ok(a: &Args, domestic: bool) -> bool {
    match a.get("Domestic") {
        None => true,
        Some(d) => d.with(|s| s == "1" || s == "true" || s == "True") == domestic,
    }
}

const RESOURCE_ROUTE_EFFECTS: [(&str, bool, &str, bool); 12] = [
    ("EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_PER_LOCAL_BONUS_RESOURCE_FOR_DOMESTIC", true, "RESOURCECLASS_BONUS", true),
    ("EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_PER_LOCAL_BONUS_RESOURCE_FOR_INTERNATIONAL", true, "RESOURCECLASS_BONUS", false),
    ("EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_PER_LOCAL_LUXURY_RESOURCE_FOR_DOMESTIC", true, "RESOURCECLASS_LUXURY", true),
    ("EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_PER_LOCAL_LUXURY_RESOURCE_FOR_INTERNATIONAL", true, "RESOURCECLASS_LUXURY", false),
    ("EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_PER_LOCAL_STRATEGIC_RESOURCE_FOR_DOMESTIC", true, "RESOURCECLASS_STRATEGIC", true),
    ("EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_PER_LOCAL_STRATEGIC_RESOURCE_FOR_INTERNATIONAL", true, "RESOURCECLASS_STRATEGIC", false),
    ("EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_PER_DESTINATION_BONUS_RESOURCE_FOR_DOMESTIC", false, "RESOURCECLASS_BONUS", true),
    ("EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_PER_DESTINATION_BONUS_RESOURCE_FOR_INTERNATIONAL", false, "RESOURCECLASS_BONUS", false),
    ("EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_PER_DESTINATION_LUXURY_RESOURCE_FOR_DOMESTIC", false, "RESOURCECLASS_LUXURY", true),
    ("EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_PER_DESTINATION_LUXURY_RESOURCE_FOR_INTERNATIONAL", false, "RESOURCECLASS_LUXURY", false),
    ("EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_PER_DESTINATION_STRATEGIC_RESOURCE_FOR_DOMESTIC", false, "RESOURCECLASS_STRATEGIC", true),
    ("EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_PER_DESTINATION_STRATEGIC_RESOURCE_FOR_INTERNATIONAL", false, "RESOURCECLASS_STRATEGIC", false),
];

/// Yields the origin city gets from one of its trade routes (yields.route_yields).
pub fn route_yields(r: &Rules, e: &Engine, route: &Route, origin: &City) -> Vec<(Sym, f64)> {
    let dest = &route.dest;
    let domestic = dest.player_id == route.origin.player_id;
    let col = if domestic { "YieldChangeAsDomesticDestination" } else { "YieldChangeAsInternationalDestination" };
    let mut y: Vec<(Sym, f64)> = vec![];
    for &t in &dest.districts {
        for (k, v) in r.yield_rows("District_TradeRouteYields", "DistrictType", t, "YieldType", col) {
            push_y(&mut y, k, v);
        }
    }
    for d in &origin.districts {
        if d.complete {
            for (k, v) in r.yield_rows("District_TradeRouteYields", "DistrictType", d.dtype, "YieldType", "YieldChangeAsOrigin") {
                push_y(&mut y, k, v);
            }
        }
    }
    let specialty = dest.districts.iter().filter(|&&t| r.district_requires_population(t)).count() as f64;
    let origin_specialty = origin.districts.iter().filter(|d| d.complete && r.district_requires_population(d.dtype)).count() as f64;
    let cs = is_city_state(r, dest.civilization);
    let techs = &e.env.techs;
    let o_center = origin.center;
    let o_plots: Vec<(Sym, Option<Sym>, Option<Sym>, Option<XY>)> = city_plots(origin).into_iter().map(|(a, b, c, xy)| (a, b, c, Some(xy))).collect();
    let d_plots: Vec<(Sym, Option<Sym>, Option<Sym>, Option<XY>)> = dest.plots.iter().map(|(a, b, c)| (*a, *b, *c, None)).collect();
    let effects = |eff: &'static str| -> Vec<Args> {
        e.player_effects(crate::sym::sym_static(eff)).into_iter().chain(e.city_effects(crate::sym::sym_static(eff), origin.name)).map(|x| x.args.clone()).collect()
    };
    let addf = |y: &mut Vec<(Sym, f64)>, eff: &'static str, times: &dyn Fn(&Args) -> f64, origin_only: bool| {
        for a in effects(eff) {
            if origin_only && is_true(&a, "Destination") && !is_true(&a, "Origin") {
                continue;
            }
            let n = times(&a);
            for (k, v) in split_args(&a, "YieldType", "Amount") {
                push_y(y, k, v * n);
            }
        }
    };
    let one = |_: &Args| 1.0;
    addf(&mut y, "EFFECT_ADJUST_TRADE_ROUTE_YIELD", &one, false);
    addf(&mut y, if domestic { "EFFECT_ADJUST_TRADE_ROUTE_YIELD_FOR_DOMESTIC" } else { "EFFECT_ADJUST_TRADE_ROUTE_YIELD_FOR_INTERNATIONAL" }, &one, false);
    addf(&mut y, if domestic { "EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_FOR_DOMESTIC" } else { "EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_FOR_INTERNATIONAL" }, &one, false);
    for (eff, local, cls, dom) in RESOURCE_ROUTE_EFFECTS {
        if dom == domestic {
            if local {
                let n = resource_count(r, &o_plots, cls, techs, Some(o_center), Some(3));
                addf(&mut y, eff, &|_| n, false);
            } else {
                let n = resource_count(r, &d_plots, cls, techs, None, None);
                addf(&mut y, eff, &|_| n, false);
            }
        }
    }
    let terr = |a: &Args| o_plots.iter().filter(|p| Some(p.0) == a.get("TerrainType")).count() as f64;
    addf(&mut y, if domestic { "EFFECT_ADJUST_PLAYER_TRADE_ROUTE_YIELD_PER_TERRAIN_FOR_DOMESTIC" } else { "EFFECT_ADJUST_PLAYER_TRADE_ROUTE_YIELD_PER_TERRAIN_FOR_INTERNATIONAL" }, &terr, true);
    let in_target = |a: &Args| d_plots.iter().filter(|p| p.2.is_some() && p.2 == a.get("ImprovementType")).count() as f64;
    addf(&mut y, "EFFECT_ADJUST_PLAYER_TRADE_ROUTE_YIELD_PER_IMPROVEMENT_IN_TARGET_CITY", &in_target, true);
    if !domestic {
        let in_origin = |a: &Args| o_plots.iter().filter(|p| p.2.is_some() && p.2 == a.get("ImprovementType")).count() as f64;
        addf(&mut y, "EFFECT_ADJUST_PLAYER_INTERNATIONAL_TRADE_ROUTE_YIELD_PER_IMPROVEMENT_IN_ORIGIN_CITY", &in_origin, false);
        addf(&mut y, "EFFECT_ADJUST_TRADE_ROUTE_YIELD_PER_SPECIALTY_DISTRICT_FOR_INTERNATIONAL", &|_| specialty, false);
        addf(&mut y, "EFFECT_ADJUST_PLAYER_TRADE_ROUTE_YIELD_PER_POST_IN_FOREIGN_CITY", &|_| 1.0, false);
    }
    let dist = hex_distance(o_center, (dest.x, dest.y)) as f64;
    addf(&mut y, "EFFECT_ADJUST_PLAYER_TRADE_ROUTE_YIELD_PER_PATH_TILE", &|_| dist, false);
    addf(&mut y, "EFFECT_ADJUST_PLAYER_TRADE_ROUTE_YIELD_PER_FOLLOWER", &|_| 0.0, false);
    if domestic {
        addf(&mut y, "EFFECT_ADJUST_TRADE_ROUTE_YIELD_PER_SPECIALTY_DISTRICT_FOR_DOMESTIC", &|_| specialty, false);
        addf(&mut y, "EFFECT_ADJUST_PLAYER_TRADE_ROUTE_YIELD_PER_POST_IN_OWN_CITY", &one, false);
    }
    if cs {
        addf(&mut y, "EFFECT_ADJUST_CITY_STATE_TRADE_ROUTE_FLAT_YIELD", &one, false);
        addf(&mut y, "EFFECT_ADJUST_CITY_STATE_TRADE_ROUTE_DISTRICT_YIELD", &|_| origin_specialty, false);
        if e.env.p.envoys.iter().any(|v| v.suzerain && Some(v.civilization) == dest.civilization) {
            addf(&mut y, "EFFECT_ADJUST_PLAYER_TRADE_ROUTE_ORIGIN_YIELD_FOR_SUZERAIN_ROUTE", &one, false);
        }
    } else if !domestic {
        for a in effects("EFFECT_ADJUST_PLAYER_PROGRESS_DIFF_TRADE_BONUS") {
            let per = a.f("TechCivicsPerYield").max(1.0) as i64;
            push_y(&mut y, s!("YIELD_SCIENCE"), ((dest.tech_count - e.env.techs.len() as i64).max(0) / per) as f64);
            push_y(&mut y, s!("YIELD_CULTURE"), ((dest.civic_count - e.env.civics.len() as i64).max(0) / per) as f64);
        }
    }
    if e.city_by_name(dest.name).is_some() {
        for x in e.city_effects(s!("EFFECT_ADJUST_TRADE_ROUTE_YIELD_TO_OTHERS"), dest.name) {
            if route_kind_ok(&x.args, domestic) {
                push_y(&mut y, x.args.get("YieldType").unwrap(), x.args.f("Amount"));
            }
        }
    }
    // a research alliance with the destination's player (alliances need Civil Service): its level-1 route yields
    // for the origin (BBG: +2 science); the alliance's later levels are not modelled
    if !domestic && dest.player_id.map(|d| e.env.allies.contains(&d)).unwrap_or(false) && e.env.civics.contains(&s!("CIVIC_CIVIL_SERVICE")) {
        for (k, v) in alliance_origin_route_yields(r) {
            push_y(&mut y, k, v);
        }
    }
    let mut pct: Vec<(Sym, f64)> = vec![];
    for a in effects("EFFECT_ADJUST_PLAYER_TRADE_ROUTE_YIELD_MODIFIER") {
        if is_true(&a, "Origin") || !is_true(&a, "Destination") {
            for (k, v) in split_args(&a, "YieldType", "Amount") {
                push_y(&mut pct, k, v);
            }
        }
    }
    if !domestic {
        for a in effects("EFFECT_ADJUST_PLAYER_INTERNATIONAL_TRADE_ROUTE_YIELD_MODIFIER") {
            for (k, v) in split_args(&a, "YieldType", "Amount") {
                push_y(&mut pct, k, v);
            }
        }
    }
    for (k, v) in pct {
        if let Some(x) = y.iter_mut().find(|(a, _)| *a == k) {
            x.1 *= 1.0 + v / 100.0;
        }
    }
    y
}

/// The level-1 research alliance's trade route yields for the origin (AllianceEffects -> ModifierArguments with AffectOrigin).
fn alliance_origin_route_yields(r: &Rules) -> Vec<(Sym, f64)> {
    let t = r.tab("AllianceEffects");
    let mut y = vec![];
    for &i in t.find("AllianceType", s!("ALLIANCE_RESEARCH")) {
        if t.v(i, "LevelRequirement").i() != 1 {
            continue;
        }
        let Some(m) = t.v(i, "ModifierID").sym() else { continue };
        let a = r.args_of("ModifierArguments", "ModifierId", m);
        if is_true(&a, "AffectOrigin") {
            if let Some(k) = a.get("YieldType") {
                push_y(&mut y, k, a.f("Amount"));
            }
        }
    }
    y
}

pub fn incoming_route_yields(e: &Engine, route: &Route, dest_city: &City) -> Vec<(Sym, f64)> {
    let domestic = route.dest.player_id == route.origin.player_id;
    let mut y = vec![];
    for x in e.city_effects(s!("EFFECT_ADJUST_TRADE_ROUTE_YIELD_FROM_OTHERS"), dest_city.name) {
        if route_kind_ok(&x.args, domestic) {
            push_y(&mut y, x.args.get("YieldType").unwrap(), x.args.f("Amount"));
        }
    }
    if !domestic {
        for x in e.city_effects(s!("EFFECT_ADJUST_CITY_YIELD_FROM_FOREIGN_TRADE_ROUTES_PASSING_THROUGH"), dest_city.name) {
            push_y(&mut y, x.args.get("YieldType").unwrap(), x.args.f("Amount"));
        }
    }
    if route.origin.player_id == e.env.p.player_id {
        let dest = |a: &Args| is_true(a, "Destination") && !is_true(a, "Origin");
        let plots = city_plots(dest_city);
        for x in e.player_effects(s!("EFFECT_ADJUST_PLAYER_TRADE_ROUTE_YIELD_PER_IMPROVEMENT_IN_TARGET_CITY")) {
            if dest(&x.args) {
                let n = plots.iter().filter(|p| p.2.is_some() && p.2 == x.args.get("ImprovementType")).count() as f64;
                push_y(&mut y, x.args.get("YieldType").unwrap(), x.args.f("Amount") * n);
            }
        }
        for x in e.player_effects(s!("EFFECT_ADJUST_PLAYER_TRADE_ROUTE_YIELD_MODIFIER")) {
            if dest(&x.args) {
                let k = x.args.get("YieldType").unwrap();
                if let Some(v) = y.iter_mut().find(|(a, _)| *a == k) {
                    v.1 *= 1.0 + x.args.f("Amount") / 100.0;
                }
            }
        }
    }
    y
}

/// city_yields: the city's total yields (before % modifiers).
pub fn city_yields(r: &Rules, e: &Engine, c: &City, routes: &[Route]) -> Y {
    let _g = crate::prof::start(1);
    let techs = &e.env.techs;
    let civics = &e.env.civics;
    let mut total: Vec<(Sym, f64)> = vec![];
    let plot_mods: &[PlotMod] = e.plot_effects.get(&c.name).map(|v| v.as_slice()).unwrap_or(&[]);
    let plots = &e.plots;
    let amods = appeal_modifiers(e, c.name);
    let mut add_all = |v: Vec<(Sym, f64)>| for (k, x) in v { push_y(&mut total, k, x) };
    for w in &c.worked {
        let is_center = w.xy() == c.center;
        let mut wp = w.clone();
        wp.district_type = if is_center { Some(s!("DISTRICT_CITY_CENTER")) } else { None };
        // a district's tile (one the save marks, or one of the city's districts the model placed) is a specialist's
        if (wp.district.map(|d| d != 0).unwrap_or(false) || c.districts.iter().any(|d| (d.x, d.y) == w.xy())) && !is_center {
            let n = if w.workers > 0 { w.workers } else { 1 };
            add_all(specialist_yields(r, c, w.xy()).into_iter().map(|(k, v)| (k, v * n as f64)).collect());
            continue;
        }
        add_all(tile_yields_bbcc(r, &wp, techs, civics, is_center, is_center && c.capital, plot_mods, Some(plots), Some(c), Some(&amods)));
        add_all(terrain_from_adjacent_improvements(e, c.name, &wp, plots));
        if !is_center {
            add_all(appeal_district_yields(r, c, &wp, plots, &amods));
        }
    }
    for &b in &c.buildings {
        add_all(r.yield_rows("Building_YieldChanges", "BuildingType", b, "YieldType", "YieldChange"));
    }
    for &(_, work) in &c.great_works {
        add_all(r.yield_rows("GreatWork_YieldChanges", "GreatWorkType", work, "YieldType", "YieldChange"));
    }
    let mut district_pct: Vec<(XY, Vec<(Sym, f64)>)> = vec![];
    for (d, x) in e.district_effects(s!("EFFECT_ADJUST_DISTRICT_YIELD_MODIFIER"), c.name) {
        let Some(d) = d else { continue };
        let k = (d.x, d.y);
        if !district_pct.iter().any(|(a, _)| *a == k) {
            district_pct.push((k, vec![]));
        }
        let v = &mut district_pct.iter_mut().find(|(a, _)| *a == k).unwrap().1;
        push_y(v, x.args.get("YieldType").unwrap(), x.args.f("Amount"));
    }
    let excluded = excluded_adjacencies(r, &e.env.p);
    for d in &c.districts {
        let adj = district_adjacency(r, d, techs, civics, Some(plots), &modifier_adjacency_rows(e, c.name, d.dtype), &excluded);
        if adj.iter().any(|(_, v)| *v != 0.0) {
            add_all(adj.clone());
            if let Some((_, pct)) = district_pct.iter().find(|(a, _)| *a == (d.x, d.y)) {
                for (k, v) in &adj {
                    let p = get_y(pct, *k);
                    if p != 0.0 {
                        let b = (v * p / 100.0).floor();
                        if b != 0.0 {
                            add_all(vec![(*k, b)]);
                        }
                    }
                }
            }
            for &b in &c.buildings {
                if r.building_in_district(b, d.dtype) {
                    let t = r.tab("Building_YieldDistrictCopies");
                    for &i in t.find("BuildingType", b) {
                        let old = t.v(i, "OldYieldType").sym().unwrap();
                        let new = t.v(i, "NewYieldType").sym().unwrap();
                        let v = get_y(&adj, old);
                        if v != 0.0 {
                            add_all(vec![(new, v)]);
                        }
                    }
                }
            }
            for (dd, x) in e.district_effects(s!("EFFECT_ADJUST_DISTRICT_YIELD_BASED_ON_ADJACENCY_BONUS"), c.name) {
                if let Some(dd) = dd {
                    if (dd.x, dd.y) == (d.x, d.y) {
                        // the adjacency as the district's percent modifiers raise it (10/21 Ravenna T62: Town
                        // Charters' +50% makes the hub's 4 gold 6, and Free Inquiry gives 6 science)
                        let boosted = |m: Sym| {
                            let v = get_y(&adj, m);
                            let p = district_pct.iter().find(|(a, _)| *a == (d.x, d.y)).map(|(_, pct)| get_y(pct, m)).unwrap_or(0.0);
                            v + (v * p / 100.0).floor()
                        };
                        let v = x.args.get("YieldTypeToMirror").map(boosted).unwrap_or(0.0);
                        if v != 0.0 {
                            add_all(vec![(x.args.get("YieldTypeToGrant").unwrap(), v)]);
                        }
                    }
                }
            }
        }
        if d.complete {
            for x in e.city_effects(s!("EFFECT_ADJUST_DISTRICT_YIELD_BASED_ON_APPEAL"), c.name) {
                if x.args.get("DistrictType").map(|t| r.district_is(d.dtype, t)).unwrap_or(false)
                    && appeal(r, &d.plot, Some(plots), Some(&amods)) >= x.args.f("RequiredAppeal") {
                    add_all(vec![(x.args.get("YieldType").unwrap(), x.args.f("YieldChange"))]);
                }
            }
        }
    }
    add_all(modifier_city_yields(r, e, c));
    for route in routes {
        if route.origin.name == c.name {
            add_all(route_yields(r, e, route, c));
        }
    }
    for route in routes {
        if route.dest.name == c.name {
            add_all(incoming_route_yields(e, route, c));
        }
    }
    let pop = c.population as f64;
    let fixed = |v: f64| (v * 256.0).floor() / 256.0;
    add_all(vec![(s!("YIELD_SCIENCE"), fixed(pop * r.param("SCIENCE_PERCENTAGE_YIELD_PER_POP") / 100.0)),
                 (s!("YIELD_CULTURE"), fixed(pop * r.param("CULTURE_PERCENTAGE_YIELD_PER_POP") / 100.0))]);
    to_y(&total)
}

/// player_yields: outside any city (prophet faith; founder beliefs need a religion, none in the plan).
pub fn player_yields(r: &Rules, e: &Engine, totals: &Y) -> Y {
    let mut y = [0.0; 6];
    y[5] += crate::gp::prophet_faith(r, e);
    for x in e.player_effects(s!("EFFECT_GRANT_YIELD_BASED_ON_CURRENT_YIELD_RATE")) {
        if let (Some(to), Some(on)) = (x.args.get("YieldToGrant"), x.args.get("YieldToBaseOn")) {
            if let Some(i) = yidx(on) {
                add(&mut y, to, totals[i] * x.args.f("Multiplier") / 100.0);
            }
        }
    }
    y
}

pub fn has_set(s: &BTreeSet<Sym>, k: Sym) -> bool {
    s.has(k)
}
