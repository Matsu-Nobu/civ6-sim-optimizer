//! The rules database (exported by python_legacy/scripts/export_problem.py): tables with rows in rowid order, and per column the
//! rows matching a value in the order SQLite returns them, so lookups see rows in the same order as the Python side.
//! `Rules` mirrors civ6sim/rules.py.
use crate::sym::{sym, Sym};
use crate::json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq)]
pub enum V {
    Null,
    I(i64),
    F(f64),
    S(Sym),
}

impl V {
    pub fn from_json(v: &Value) -> V {
        match v {
            Value::Null => V::Null,
            Value::Bool(b) => V::I(*b as i64),
            Value::Num(n) => {
                if n.fract() == 0.0 && n.abs() < 9e15 {
                    V::I(*n as i64)
                } else {
                    V::F(*n)
                }
            }
            Value::Str(s) => V::S(sym(s)),
            _ => V::Null,
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, V::Null)
    }
    /// Python truthiness of the SQLite value (0, 0.0, '' and NULL are false).
    pub fn truthy(&self) -> bool {
        match self {
            V::Null => false,
            V::I(i) => *i != 0,
            V::F(f) => *f != 0.0,
            V::S(s) => !s.is_empty(),
        }
    }
    pub fn sym(&self) -> Option<Sym> {
        match self {
            V::S(s) if !s.is_empty() => Some(*s),
            V::S(_) => Some(Sym(0)),
            _ => None,
        }
    }
    pub fn f(&self) -> f64 {
        match self {
            V::I(i) => *i as f64,
            V::F(f) => *f,
            V::S(s) => s.with(|x| x.trim().parse::<f64>().unwrap_or(0.0)),
            V::Null => 0.0,
        }
    }
    pub fn i(&self) -> i64 {
        match self {
            V::I(i) => *i,
            V::F(f) => *f as i64,
            V::S(s) => s.with(|x| x.trim().parse::<f64>().map(|f| f as i64).unwrap_or(0)),
            V::Null => 0,
        }
    }
    fn key(&self) -> Option<K> {
        match self {
            V::Null => None,
            V::I(i) => Some(K::I(*i)),
            V::F(f) => Some(K::F(f.to_bits())),
            V::S(s) => Some(K::S(*s)),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum K {
    I(i64),
    F(u64),
    S(Sym),
}

impl From<Sym> for K {
    fn from(s: Sym) -> K {
        K::S(s)
    }
}
impl From<i64> for K {
    fn from(i: i64) -> K {
        K::I(i)
    }
}

pub struct Table {
    pub cols: HashMap<String, usize>,
    exact: HashMap<String, usize>,
    pub rows: Vec<Vec<V>>,
    order: Vec<HashMap<K, Vec<u32>>>,
}

static EMPTY: Vec<u32> = Vec::new();

impl Table {
    fn load(v: &Value) -> Table {
        let cols: Vec<String> = v["cols"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().to_string()).collect();
        let rows: Vec<Vec<V>> = v["rows"].as_array().unwrap().iter()
            .map(|r| r.as_array().unwrap().iter().map(V::from_json).collect()).collect();
        let mut order = Vec::with_capacity(cols.len());
        for (ci, c) in cols.iter().enumerate() {
            let mut m: HashMap<K, Vec<u32>> = HashMap::new();
            if let Some(by) = v["order"].get(c).and_then(|x| x.as_object()) {
                for (_, idx) in by {
                    let idx: Vec<u32> = idx.as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u32).collect();
                    if let Some(k) = idx.first().and_then(|&i| rows[i as usize][ci].key()) {
                        m.insert(k, idx);
                    }
                }
            } else {
                // a unique column (or one the export skipped): rowid order
                for (i, r) in rows.iter().enumerate() {
                    if let Some(k) = r[ci].key() {
                        m.entry(k).or_default().push(i as u32);
                    }
                }
            }
            order.push(m);
        }
        Table { exact: cols.iter().enumerate().map(|(i, c)| (c.clone(), i)).collect(),
                cols: cols.into_iter().enumerate().map(|(i, c)| (c.to_lowercase(), i)).collect(), rows, order }
    }
    pub fn col(&self, name: &str) -> usize {
        if let Some(&i) = self.exact.get(name) {
            return i;
        }
        *self.cols.get(&name.to_lowercase()).unwrap_or_else(|| panic!("no column {name}"))
    }
    pub fn has_col(&self, name: &str) -> bool {
        self.exact.contains_key(name) || self.cols.contains_key(&name.to_lowercase())
    }
    /// Rows where `col` = key, in SQLite's order.
    pub fn find(&self, col: &str, key: impl Into<K>) -> &[u32] {
        let c = self.col(col);
        self.order[c].get(&key.into()).map(|v| v.as_slice()).unwrap_or(&EMPTY)
    }
    pub fn v(&self, row: u32, col: &str) -> &V {
        &self.rows[row as usize][self.col(col)]
    }
    /// The first row where col = key.
    pub fn first(&self, col: &str, key: impl Into<K>) -> Option<u32> {
        self.find(col, key).first().copied()
    }
    pub fn len(&self) -> usize {
        self.rows.len()
    }
}

pub const YIELDS: [&str; 6] = ["YIELD_FOOD", "YIELD_PRODUCTION", "YIELD_GOLD", "YIELD_SCIENCE", "YIELD_CULTURE", "YIELD_FAITH"];

/// Yield vector indexed like YIELDS.
pub type Y = [f64; 6];

pub fn yidx(y: Sym) -> Option<usize> {
    let ys = ysyms();
    ys.iter().position(|&s| s == y)
}

pub fn ysyms() -> &'static [Sym; 6] {
    static S: OnceLock<[Sym; 6]> = OnceLock::new();
    S.get_or_init(|| YIELDS.map(sym))
}

/// A requirement: (type, args, inverse).
#[derive(Clone, Debug)]
pub struct Req {
    pub rtype: Sym,
    pub args: Args,
    pub inverse: bool,
}

/// Modifier / requirement arguments (Name -> Value, both text in the database).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Args(pub Vec<(Sym, Sym)>);

impl Args {
    pub fn get(&self, name: &'static str) -> Option<Sym> {
        let n = crate::sym::sym_static(name);
        self.0.iter().find(|(k, _)| *k == n).map(|(_, v)| *v)
    }
    pub fn gs(&self, n: Sym) -> Option<Sym> {
        self.0.iter().find(|(k, _)| *k == n).map(|(_, v)| *v)
    }
    pub fn f(&self, name: &'static str) -> f64 {
        self.get(name).map(|v| v.with(|s| s.trim().parse::<f64>().unwrap_or(0.0))).unwrap_or(0.0)
    }
    /// float(a.get(name) or 0): missing or empty -> 0
    pub fn f_or0(&self, name: &'static str) -> f64 {
        self.f(name)
    }
    pub fn is(&self, name: &'static str, vals: &[&str]) -> bool {
        self.get(name).map(|v| v.with(|s| vals.contains(&s))).unwrap_or(false)
    }
    pub fn has(&self, name: &'static str) -> bool {
        self.get(name).is_some()
    }
}

pub struct ModRow {
    pub mtype: Sym,
    pub subj_rs: Option<Sym>,
    pub owner_rs: Option<Sym>,
    pub collection: Option<Sym>,
    pub effect: Option<Sym>,
    pub args: Args,
    pub permanent: bool,
}

pub struct Rules {
    pub imp_cache: OnceLock<Vec<crate::placement::ImpInfo>>,
    pub t: HashMap<String, Table>,
    scans: HashMap<String, Vec<u32>>,
    params: HashMap<String, Sym>,
    // memoized derived lookups
    reqsets: HashMap<Sym, std::sync::Arc<(Vec<Req>, bool)>>,
    mods: HashMap<Sym, std::sync::Arc<ModRow>>,
    pub eras: Vec<Sym>,
    pub n_tech: usize,
    pub n_civic: usize,
}

impl Rules {
    pub fn load(v: &Value) -> Rules {
        let mut t = HashMap::new();
        let mut scans = HashMap::new();
        for (name, tv) in v.as_object().unwrap() {
            if name == "__scans__" {
                for (k, idx) in tv.as_object().unwrap() {
                    scans.insert(k.clone(), idx.as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u32).collect());
                }
                continue;
            }
            t.insert(name.clone(), Table::load(tv));
        }
        let mut r = Rules {
            imp_cache: OnceLock::new(),
            t,
            scans,
            params: HashMap::new(),
            reqsets: HashMap::new(),
            mods: HashMap::new(),
            eras: vec![],
            n_tech: 0,
            n_civic: 0,
        };
        let gp = &r.t["GlobalParameters"];
        let (cn, cv) = (gp.col("Name"), gp.col("Value"));
        let params: HashMap<String, Sym> = gp.rows.iter().map(|row| {
            let name = row[cn].sym().unwrap().as_string();
            let val = match &row[cv] {
                V::S(s) => *s,
                V::I(i) => sym(&i.to_string()),
                V::F(f) => sym(&f.to_string()),
                V::Null => Sym(0),
            };
            (name, val)
        }).collect();
        r.params = params;
        let eras = &r.t["Eras"];
        let mut e: Vec<(i64, Sym)> = eras.rows.iter().map(|row| (row[eras.col("ChronologyIndex")].i(), row[eras.col("EraType")].sym().unwrap())).collect();
        e.sort();
        r.eras = e.into_iter().map(|(_, s)| s).collect();
        r.n_tech = r.t["Technologies"].len();
        r.n_civic = r.t["Civics"].len();
        // every requirement set and modifier, read once (no locks while simulating)
        let rs = r.tab("RequirementSets");
        let ids: Vec<Sym> = (0..rs.len() as u32).filter_map(|i| rs.v(i, "RequirementSetId").sym()).collect();
        let sets: HashMap<Sym, std::sync::Arc<(Vec<Req>, bool)>> = ids.into_iter().map(|id| (id, std::sync::Arc::new(r.read_requirement_set(id)))).collect();
        let m = r.tab("Modifiers");
        let mids: Vec<Sym> = (0..m.len() as u32).filter_map(|i| m.v(i, "ModifierId").sym()).collect();
        let mods: HashMap<Sym, std::sync::Arc<ModRow>> = mids.into_iter().filter_map(|id| r.read_modifier(id).map(|x| (id, std::sync::Arc::new(x)))).collect();
        r.reqsets = sets;
        r.mods = mods;
        r
    }

    /// Row numbers of a whole-table read in the order the Python side saw them (see export_problem.SCANS); rowid
    /// order when not exported. Rows outside the read's WHERE are not in an exported list.
    pub fn scan(&self, name: &str, table: &str) -> Vec<u32> {
        match self.scans.get(name) {
            Some(v) => v.clone(),
            None => (0..self.tab(table).len() as u32).collect(),
        }
    }

    pub fn tab(&self, name: &str) -> &Table {
        self.t.get(name).unwrap_or_else(|| panic!("no table {name}"))
    }
    pub fn has_table(&self, name: &str) -> bool {
        self.t.contains_key(name)
    }

    pub fn param(&self, name: &str) -> f64 {
        self.params.get(name).unwrap_or_else(|| panic!("no param {name}")).with(|s| s.parse::<f64>().unwrap())
    }

    pub fn era_index(&self, era: Sym) -> Option<usize> {
        self.eras.iter().position(|&e| e == era)
    }

    /// First value of `col` in table where key_col = key.
    pub fn lookup(&self, table: &str, key_col: &str, key: impl Into<K>, col: &str) -> Option<&V> {
        let t = self.tab(table);
        t.first(key_col, key).map(|i| t.v(i, col))
    }
    pub fn lookup_sym(&self, table: &str, key_col: &str, key: Sym, col: &str) -> Option<Sym> {
        self.lookup(table, key_col, key, col).and_then(|v| v.sym()).filter(|s| !s.is_empty())
    }
    pub fn lookup_f(&self, table: &str, key_col: &str, key: Sym, col: &str) -> f64 {
        self.lookup(table, key_col, key, col).map(|v| v.f()).unwrap_or(0.0)
    }
    pub fn lookup_truthy(&self, table: &str, key_col: &str, key: Sym, col: &str) -> bool {
        self.lookup(table, key_col, key, col).map(|v| v.truthy()).unwrap_or(false)
    }
    /// [col values] of rows where key_col = key, in order.
    pub fn list_sym(&self, table: &str, key_col: &str, key: Sym, col: &str) -> Vec<Sym> {
        let t = self.tab(table);
        let c = t.col(col);
        t.find(key_col, key).iter().filter_map(|&i| t.rows[i as usize][c].sym()).collect()
    }
    /// (yield, amount) rows of a yield table for a key.
    pub fn yield_rows(&self, table: &str, key_col: &str, key: Sym, ycol: &str, vcol: &str) -> Vec<(Sym, f64)> {
        let t = self.tab(table);
        let (yc, vc) = (t.col(ycol), t.col(vcol));
        t.find(key_col, key).iter().map(|&i| (t.rows[i as usize][yc].sym().unwrap(), t.rows[i as usize][vc].f())).collect()
    }

    pub fn game_speed_cost_multiplier(&self, speed: Sym) -> f64 {
        self.lookup_f("GameSpeeds", "GameSpeedType", speed, "CostMultiplier") / 100.0
    }

    /// select Name, Value from ModifierArguments / RequirementArguments where Id=?
    pub fn args_of(&self, table: &str, id_col: &str, id: Sym) -> Args {
        let t = self.tab(table);
        let (n, v) = (t.col("Name"), t.col("Value"));
        let mut a: Vec<(Sym, Sym)> = Vec::new();
        for &i in t.find(id_col, id) {
            let name = t.rows[i as usize][n].sym().unwrap();
            let val = match &t.rows[i as usize][v] {
                V::S(s) => *s,
                V::I(x) => sym(&x.to_string()),
                V::F(x) => sym(&py_float_str(*x)),
                V::Null => Sym(0),
            };
            // dict(): a later duplicate name replaces the earlier value, keeping the first position
            if let Some(p) = a.iter_mut().find(|(k, _)| *k == name) {
                p.1 = val;
            } else {
                a.push((name, val));
            }
        }
        Args(a)
    }

    /// ([requirements], test_any) of a requirement set (rules.requirement_set).
    pub fn requirement_set(&self, rs: Sym) -> std::sync::Arc<(Vec<Req>, bool)> {
        if let Some(x) = self.reqsets.get(&rs) {
            return x.clone();
        }
        std::sync::Arc::new(self.read_requirement_set(rs))
    }

    fn read_requirement_set(&self, rs: Sym) -> (Vec<Req>, bool) {
        let rst = self.tab("RequirementSets");
        let test_any = rst.first("RequirementSetId", rs)
            .map(|i| rst.v(i, "RequirementSetType").sym() == Some(sym("REQUIREMENTSET_TEST_ANY"))).unwrap_or(false);
        let s = self.tab("RequirementSetRequirements");
        let rq = self.tab("Requirements");
        let mut reqs = Vec::new();
        for &i in s.find("RequirementSetId", rs) {
            let rid = s.v(i, "RequirementId").sym().unwrap();
            for &j in rq.find("RequirementId", rid) {
                reqs.push(Req {
                    rtype: rq.v(j, "RequirementType").sym().unwrap(),
                    args: self.args_of("RequirementArguments", "RequirementId", rid),
                    inverse: rq.v(j, "Inverse").truthy(),
                });
            }
        }
        (reqs, test_any)
    }

    /// Modifier row joined with DynamicModifiers, its arguments and Permanent flag.
    pub fn modifier(&self, mid: Sym) -> Option<std::sync::Arc<ModRow>> {
        self.mods.get(&mid).cloned()
    }

    fn read_modifier(&self, mid: Sym) -> Option<ModRow> {
        let m = self.tab("Modifiers");
        m.first("ModifierId", mid).map(|i| {
            let mtype = m.v(i, "ModifierType").sym().unwrap();
            let dm = self.tab("DynamicModifiers");
            let (collection, effect) = match dm.first("ModifierType", mtype) {
                Some(j) => (dm.v(j, "CollectionType").sym(), dm.v(j, "EffectType").sym()),
                None => (None, None),
            };
            ModRow {
                mtype,
                subj_rs: m.v(i, "SubjectRequirementSetId").sym().filter(|s| !s.is_empty()),
                owner_rs: m.v(i, "OwnerRequirementSetId").sym().filter(|s| !s.is_empty()),
                collection,
                effect,
                args: self.args_of("ModifierArguments", "ModifierId", mid),
                permanent: m.v(i, "Permanent").truthy(),
            }
        })
    }

    /// The civilization's and leader's traits, with inherited leaders (rules.traits_of). Python returns a set;
    /// here: civilization traits, then each leader's in inheritance order, without duplicates.
    pub fn traits_of(&self, civ: Option<Sym>, leader: Option<Sym>) -> std::sync::Arc<Vec<Sym>> {
        thread_local! {
            static TRAITS: std::cell::RefCell<HashMap<(Sym, Sym), std::sync::Arc<Vec<Sym>>>> = std::cell::RefCell::new(HashMap::new());
        }
        let key = (civ.unwrap_or(Sym(0)), leader.unwrap_or(Sym(0)));
        if let Some(x) = TRAITS.with(|t| t.borrow().get(&key).cloned()) {
            return x;
        }
        let mut out: Vec<Sym> = Vec::new();
        let push = |t: Sym, out: &mut Vec<Sym>| if !out.contains(&t) { out.push(t) };
        if let Some(c) = civ {
            for t in self.list_sym("CivilizationTraits", "CivilizationType", c, "TraitType") {
                push(t, &mut out);
            }
        }
        let mut seen = HashSet::new();
        let mut l = leader;
        while let Some(ld) = l {
            if ld.is_empty() || !seen.insert(ld) {
                break;
            }
            for t in self.list_sym("LeaderTraits", "LeaderType", ld, "TraitType") {
                push(t, &mut out);
            }
            l = self.lookup_sym("Leaders", "LeaderType", ld, "InheritFrom");
        }
        let a = std::sync::Arc::new(out);
        TRAITS.with(|t| t.borrow_mut().insert(key, a.clone()));
        a
    }
    pub fn civ_has_trait(&self, civ: Option<Sym>, leader: Option<Sym>, tr: Sym) -> bool {
        self.traits_of(civ, leader).contains(&tr)
    }

    fn replaced(&self, table: &str, col: &str, item: Sym, civ: Option<Sym>, leader: Option<Sym>) -> bool {
        let t = self.tab(table);
        let uc = t.col(&format!("CivUnique{col}Type"));
        for &i in t.find(&format!("Replaces{col}Type"), item) {
            let uniq = t.rows[i as usize][uc].sym().unwrap();
            if let Some(tr) = self.lookup_sym(&format!("{col}s"), &format!("{col}Type"), uniq, "TraitType") {
                if self.civ_has_trait(civ, leader, tr) {
                    return true;
                }
            }
        }
        false
    }
    pub fn unit_replaced(&self, u: Sym, civ: Option<Sym>, leader: Option<Sym>) -> bool {
        self.replaced("UnitReplaces", "Unit", u, civ, leader)
    }
    pub fn building_replaced(&self, b: Sym, civ: Option<Sym>, leader: Option<Sym>) -> bool {
        self.replaced("BuildingReplaces", "Building", b, civ, leader)
    }
    pub fn district_replaced(&self, d: Sym, civ: Option<Sym>, leader: Option<Sym>) -> bool {
        self.replaced("DistrictReplaces", "District", d, civ, leader)
    }
    /// A unique district counts as the base district it replaces.
    pub fn district_replaces_done(&self, district: Sym, done: &[Sym]) -> bool {
        let t = self.tab("DistrictReplaces");
        done.iter().any(|&d| t.find("CivUniqueDistrictType", d).iter().any(|&i| t.v(i, "ReplacesDistrictType").sym() == Some(district)))
    }
    pub fn building_prereqs_met(&self, b: Sym, buildings: &[Sym]) -> bool {
        let pre = self.list_sym("BuildingPrereqs", "Building", b, "PrereqBuilding");
        pre.is_empty() || pre.iter().any(|x| buildings.contains(x))
    }
    pub fn unit_era(&self, unit: Sym) -> Sym {
        if let Some(pt) = self.lookup_sym("Units", "UnitType", unit, "PrereqTech") {
            return self.lookup_sym("Technologies", "TechnologyType", pt, "EraType").unwrap();
        }
        if let Some(pc) = self.lookup_sym("Units", "UnitType", unit, "PrereqCivic") {
            return self.lookup_sym("Civics", "CivicType", pc, "EraType").unwrap();
        }
        sym("ERA_ANCIENT")
    }
    pub fn has_boost(&self, item: Sym) -> bool {
        let col = if item.starts_with("TECH_") { "TechnologyType" } else { "CivicType" };
        !self.tab("Boosts").find(col, item).is_empty()
    }
    pub fn boost_class(&self, item: Sym) -> Option<Sym> {
        let col = if item.starts_with("TECH_") { "TechnologyType" } else { "CivicType" };
        self.lookup_sym("Boosts", col, item, "BoostClass")
    }
    pub fn valid_improvements(&self, resource: Sym) -> Vec<Sym> {
        self.list_sym("Improvement_ValidResources", "ResourceType", resource, "ImprovementType")
    }
    pub fn resource_revealed(&self, resource: Sym, techs: &impl Has) -> bool {
        match self.lookup("Resources", "ResourceType", resource, "PrereqTech") {
            None => true,
            Some(v) => match v.sym() {
                None => true,
                Some(t) if t.is_empty() => true,
                Some(t) => techs.has(t),
            },
        }
    }
    pub fn resource_class(&self, resource: Sym) -> Option<Sym> {
        self.lookup_sym("Resources", "ResourceType", resource, "ResourceClassType")
    }
    pub fn is_natural_wonder(&self, feature: Sym) -> bool {
        self.lookup_truthy("Features", "FeatureType", feature, "NaturalWonder")
    }
    pub fn district_is(&self, district: Sym, base: Sym) -> bool {
        district == base || {
            let t = self.tab("DistrictReplaces");
            t.find("CivUniqueDistrictType", district).iter().any(|&i| t.v(i, "ReplacesDistrictType").sym() == Some(base))
        }
    }
    pub fn building_in_district(&self, b: Sym, district: Sym) -> bool {
        match self.lookup_sym("Buildings", "BuildingType", b, "PrereqDistrict") {
            Some(pd) => self.district_is(district, pd) || self.district_is(pd, district),
            None => false,
        }
    }
    pub fn building_is(&self, b: Sym, base: Sym) -> bool {
        b == base || {
            let t = self.tab("BuildingReplaces");
            t.find("CivUniqueBuildingType", b).iter().any(|&i| t.v(i, "ReplacesBuildingType").sym() == Some(base))
        }
    }
    pub fn city_has_building(&self, buildings: &[Sym], base: Sym) -> bool {
        buildings.iter().any(|&b| self.building_is(b, base))
    }
    pub fn is_wonder(&self, b: Sym) -> bool {
        self.lookup_truthy("Buildings", "BuildingType", b, "IsWonder")
    }
    pub fn terrain_class(&self, terrain: Sym) -> Option<Sym> {
        self.lookup_sym("TerrainClass_Terrains", "TerrainType", terrain, "TerrainClassType")
    }
    pub fn district_requires_population(&self, d: Sym) -> bool {
        self.lookup_truthy("Districts", "DistrictType", d, "RequiresPopulation")
    }
    pub fn tags_of(&self, t: Option<Sym>) -> Vec<Sym> {
        match t {
            Some(t) if !t.is_empty() => self.list_sym("TypeTags", "Type", t, "Tag"),
            _ => vec![],
        }
    }
    pub fn cost(&self, item: Sym) -> f64 {
        let (table, col) = match item.with(|s| s.split('_').next().unwrap_or("").to_string()).as_str() {
            "UNIT" => ("Units", "UnitType"),
            "BUILDING" => ("Buildings", "BuildingType"),
            "DISTRICT" => ("Districts", "DistrictType"),
            _ => ("Projects", "ProjectType"),
        };
        self.lookup_f(table, col, item, "Cost")
    }
}

/// Python's str(float) for the values the code compares as text (e.g. 1.0 -> "1.0").
pub fn py_float_str(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e16 {
        format!("{:.1}", x)
    } else {
        format!("{}", x)
    }
}

/// Membership test used for tech / civic sets.
pub trait Has {
    fn has(&self, s: Sym) -> bool;
}
impl Has for HashSet<Sym> {
    fn has(&self, s: Sym) -> bool {
        self.contains(&s)
    }
}
impl Has for Vec<Sym> {
    fn has(&self, s: Sym) -> bool {
        self.contains(&s)
    }
}
impl Has for std::collections::BTreeSet<Sym> {
    fn has(&self, s: Sym) -> bool {
        self.contains(&s)
    }
}
