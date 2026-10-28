//! Reading a .Civ6Save: the game state as the export (save/export.rs) needs it.
//!
//! Ported from civ6-savegame-parser (github.com/bestander/civ6-savegame-parser, MIT), with its header walker
//! vendored from pydt/civ6-save-parser (MIT); see data/save/NOTICE. Each reader keeps the layout notes of the
//! original. Only the parts the export reads are ported.
//!
//! A reader that runs past the end of the payload panics, as the original's Buffer reads throw; the export then
//! skips the save (export.rs).
pub mod export;
pub mod header;

use std::collections::HashMap;
use std::sync::OnceLock;

// ---- bytes ---------------------------------------------------------------------------------------------------

/// Little-endian reads at a signed offset; out of range panics (Buffer.readInt32LE throws a RangeError).
pub struct Bytes<'a>(pub &'a [u8]);

impl<'a> Bytes<'a> {
    pub fn len(&self) -> i64 {
        self.0.len() as i64
    }
    fn at(&self, o: i64, n: i64) -> &'a [u8] {
        if o < 0 || o + n > self.len() {
            panic!("read of {n} bytes at {o} outside the payload ({})", self.len());
        }
        &self.0[o as usize..(o + n) as usize]
    }
    pub fn u8(&self, o: i64) -> u8 {
        self.at(o, 1)[0]
    }
    pub fn i8(&self, o: i64) -> i8 {
        self.at(o, 1)[0] as i8
    }
    pub fn u16(&self, o: i64) -> u16 {
        u16::from_le_bytes(self.at(o, 2).try_into().unwrap())
    }
    pub fn i16(&self, o: i64) -> i16 {
        i16::from_le_bytes(self.at(o, 2).try_into().unwrap())
    }
    pub fn u32(&self, o: i64) -> u32 {
        u32::from_le_bytes(self.at(o, 4).try_into().unwrap())
    }
    pub fn i32(&self, o: i64) -> i32 {
        i32::from_le_bytes(self.at(o, 4).try_into().unwrap())
    }
    /// Buffer.indexOf: the first match at or after `from` (a negative `from` counts from the end), or -1.
    pub fn index_of(&self, needle: &[u8], from: i64) -> i64 {
        index_of(self.0, needle, from)
    }
    /// The bytes [a, b) clamped to the payload (Buffer.subarray).
    pub fn sub(&self, a: i64, b: i64) -> &'a [u8] {
        let a = a.clamp(0, self.len()) as usize;
        let b = (b.clamp(0, self.len()) as usize).max(a);
        &self.0[a..b]
    }
}

pub fn index_of(hay: &[u8], needle: &[u8], from: i64) -> i64 {
    let n = hay.len() as i64;
    let from = if from < 0 { (n + from).max(0) } else { from };
    if needle.is_empty() || from > n {
        return if needle.is_empty() { from.min(n) } else { -1 };
    }
    let first = needle[0];
    let mut i = from as usize;
    let last = hay.len().saturating_sub(needle.len());
    while i <= last {
        match hay[i..=last].iter().position(|&b| b == first) {
            Some(k) => {
                i += k;
                if &hay[i..i + needle.len()] == needle {
                    return i as i64;
                }
                i += 1;
            }
            None => return -1,
        }
    }
    -1
}

pub fn last_index_of(hay: &[u8], needle: &[u8]) -> i64 {
    if needle.len() > hay.len() {
        return -1;
    }
    (0..=hay.len() - needle.len()).rev().find(|&i| &hay[i..i + needle.len()] == needle).map(|i| i as i64).unwrap_or(-1)
}

/// Latin-1 text of bytes (Buffer.toString('latin1')).
pub fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

// ---- decompression -------------------------------------------------------------------------------------------

/// The zlib payload of a save: from the zlib header after the last MOD_TITLE marker to the last `00 00 FF FF`,
/// stored in 64 KiB chunks with 4 bytes between them; inflated as far as the data goes (Z_SYNC_FLUSH).
pub fn decompress(save: &[u8]) -> Vec<u8> {
    let mod_title = [0x72, 0xe1, 0x34, 0x30];
    let from = last_index_of(save, &mod_title).max(0);
    let start = index_of(save, &[0x78, 0x9c], from);
    assert!(start >= 0, "Civ6Save: zlib header not found in file");
    let end = last_index_of(save, &[0, 0, 0xff, 0xff]);
    assert!(end >= start, "Civ6Save: compressed region end marker not found");
    let data = &save[start as usize..end as usize + 4];
    let mut joined = Vec::with_capacity(data.len());
    let mut pos = 0;
    while pos < data.len() {
        joined.extend_from_slice(&data[pos..(pos + 65536).min(data.len())]);
        pos += 65536 + 4;
    }
    match miniz_oxide::inflate::decompress_to_vec_zlib(&joined) {
        Ok(v) => v,
        // the stream ends at a sync flush, without a final block: keep what came out
        Err(e) => e.output,
    }
}

// ---- type names ----------------------------------------------------------------------------------------------

/// Civ6's hash of a type name: CRC-32 without the final complement.
pub fn civ6_hash(name: &str) -> u32 {
    static TABLE: OnceLock<[u32; 256]> = OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (n, e) in t.iter_mut().enumerate() {
            let mut c = n as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { (c >> 1) ^ 0xedb88320 } else { c >> 1 };
            }
            *e = c;
        }
        t
    });
    let mut crc = 0xffffffffu32;
    for b in name.bytes() {
        crc = (crc >> 8) ^ t[((crc ^ b as u32) & 0xff) as usize];
    }
    crc
}

pub fn bswap32(h: u32) -> u32 {
    h.swap_bytes()
}

/// A database type: its name and kind (`UNIT_WARRIOR`, `KIND_UNIT`).
#[derive(Clone, Copy)]
pub struct Type {
    pub name: &'static str,
    pub kind: &'static str,
}

/// Every type name by hash: the bundled dictionary (data/save/civ6-types.json, from the game's XML) and the
/// rules database's Types (mod additions); hash-tables.json names the plot fields the dictionary lacks.
pub struct Dict {
    by_hash: HashMap<u32, Type>,
    terrains: HashMap<String, String>,
    features: HashMap<String, String>,
    resources: HashMap<String, String>,
    improvements: HashMap<String, String>,
    instance_types: HashMap<String, String>,
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

impl Dict {
    /// `extra`: (name, kind) pairs added after the bundled ones; a name already listed under its kind is kept.
    pub fn new(extra: &[(String, String)]) -> Dict {
        let types = crate::json::parse(include_str!("../../data/save/civ6-types.json")).unwrap();
        let mut kinds: Vec<(String, Vec<String>)> = types.as_object().unwrap().iter()
            .map(|(k, v)| (k.clone(), v.as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect())).collect();
        for (name, kind) in extra {
            let i = match kinds.iter().position(|(k, _)| k == kind) {
                Some(i) => i,
                None => {
                    kinds.push((kind.clone(), vec![]));
                    kinds.len() - 1
                }
            };
            if !kinds[i].1.contains(name) {
                kinds[i].1.push(name.clone());
            }
        }
        // later entries win on a shared hash, as the original's Map.set does
        let mut by_hash = HashMap::new();
        for (kind, names) in kinds {
            let kind = leak(kind);
            for n in names {
                by_hash.insert(civ6_hash(&n), Type { name: leak(n), kind });
            }
        }
        let tables = crate::json::parse(include_str!("../../data/save/hash-tables.json")).unwrap();
        let names = |k: &str, field: Option<&str>| -> HashMap<String, String> {
            tables[k].as_object().map(|o| o.iter().map(|(h, v)| (h.clone(), match field {
                Some(f) => v[f].as_str().unwrap_or("").to_string(),
                None => v.as_str().unwrap_or("").to_string(),
            })).collect()).unwrap_or_default()
        };
        Dict {
            by_hash,
            terrains: names("terrains", Some("name")),
            features: names("features", None),
            resources: names("resources", None),
            improvements: names("improvements", None),
            instance_types: names("instanceTypes", None),
        }
    }
    /// The type behind a hash in either byte order, or None (also for 0 and -1).
    pub fn resolve(&self, h: u32) -> Option<Type> {
        if h == 0xffffffff || h == 0 {
            return None;
        }
        self.by_hash.get(&h).or_else(|| self.by_hash.get(&bswap32(h))).copied()
    }
    pub fn of_kind(&self, h: u32, kind: &str) -> Option<&'static str> {
        self.resolve(h).filter(|t| t.kind == kind).map(|t| t.name)
    }
    /// The type at a hash only (no byte swap): the typed-table scan's lookup.
    fn exact(&self, h: u32) -> Option<Type> {
        self.by_hash.get(&h).copied()
    }
    fn table(t: &HashMap<String, String>, h: u32) -> Option<String> {
        if h == 0xffffffff {
            return None;
        }
        t.get(&format!("{h:08x}")).or_else(|| t.get(&format!("{:08x}", bswap32(h)))).cloned()
    }
    /// A plot field's name: the type dictionary, else the sheet table.
    fn plot_name(&self, h: u32, table: &HashMap<String, String>, kind: &str) -> Option<String> {
        if h == 0xffffffff || h == 0 {
            return Dict::table(table, h).or_else(|| self.of_kind(h, kind).map(String::from));
        }
        self.resolve(h).map(|t| t.name.to_string()).or_else(|| Dict::table(table, h).or_else(|| self.of_kind(h, kind).map(String::from)))
    }
    fn instance_type(&self, h: u32) -> Option<String> {
        self.resolve(h).map(|t| t.name.to_string()).or_else(|| Dict::table(&self.instance_types, h))
    }
}

// ---- the map -------------------------------------------------------------------------------------------------

#[derive(Clone)]
pub struct Owner {
    pub player_id: i64,
    pub city_slot: i64,
    pub district_instance_id: Option<u32>,
    pub world_wonder: Option<String>,
}

#[derive(Clone)]
pub struct Plot {
    pub index: i64,
    pub x: i64,
    pub y: i64,
    pub terrain: String,
    pub feature: Option<String>,
    pub resource: Option<String>,
    pub resource_hash: u32,
    pub improvement: Option<String>,
    pub route_index: Option<i64>,
    pub pillaged: bool,
    pub river_map: u8,
    pub yield_changes: Option<Vec<(String, f64)>>,
    pub owner: Option<Owner>,
    /// The raw district instance id (0xffffffff: none), for the city groups.
    district_raw: u32,
}

pub struct Map {
    pub width: i64,
    pub height: i64,
    pub plots: Vec<Plot>,
    pub section_start: i64,
}

const TILE_SECTION_MARKER: [u8; 12] = [0x0e, 0, 0, 0, 0x0f, 0, 0, 0, 0x06, 0, 0, 0];
/// Next-tile terrain hashes used to find where the next tile record starts.
const TERRAIN_RESYNC: [u32; 17] = [2213004848, 1855786096, 1602466867, 4226188894, 3872285854, 2746853616, 3852995116,
    3108058291, 1418772217, 1223859883, 3949113590, 3746160061, 1743422479, 3842183808, 699483892, 1248885265, 1204357597];

fn parse_map(p: &Bytes, d: &Dict) -> Map {
    // The marker can occur by chance before the real tile section: take the first occurrence whose tile count is a
    // whole number of map rows.
    let mut start = p.index_of(&TILE_SECTION_MARKER, 0);
    assert!(start >= 0, "Civ6Save: tile section marker not found in payload");
    let mut count = 0i64;
    let mut width = 0i64;
    while start >= 0 {
        count = p.u32(start + 12) as i64;
        if (100..=100000).contains(&count) {
            let mut needle = vec![0u8, 0, 0, 0];
            needle.extend_from_slice(p.sub(start + 12, start + 16));
            width = 0;
            let mut sp = -1;
            while width <= 0 {
                sp = p.index_of(&needle, sp + 1);
                if sp < 0 {
                    break;
                }
                width = p.i16(sp + 8) as i64;
            }
            if width > 0 && count % width == 0 {
                break;
            }
        }
        start = p.index_of(&TILE_SECTION_MARKER, start + 1);
    }
    assert!(start >= 0 && width > 0 && count % width == 0, "Civ6Save: no tile section with a tile count divisible by the map width");
    let mut cursor = start + 16;
    let mut plots = Vec::with_capacity(count as usize);
    for i in 0..count {
        let base = cursor;
        let terrain_h = p.u32(base + 12);
        let feature_h = p.u32(base + 16);
        let resource_h = p.u32(base + 27);
        let improvement_h = p.u32(base + 33);
        let overlay = p.u32(base + 51);
        cursor += 55;
        cursor += match overlay {
            1 => 24,
            2 => 44,
            3 => 64,
            _ => 0,
        };
        let mut owner = None;
        let mut district_raw = 0xffffffff;
        if p.u8(base + 49) & 0x40 != 0 {
            let wonder = p.u32(cursor + 13);
            district_raw = p.u32(cursor + 8);
            owner = Some(Owner {
                player_id: p.u8(cursor + 12) as i64,
                city_slot: p.u16(cursor) as i64,
                district_instance_id: (district_raw != 0xffffffff).then_some(district_raw),
                world_wonder: if wonder == 0xffffffff { None } else { d.resolve(wonder).map(|t| t.name.to_string()) },
            });
            cursor += 17;
        }
        if i < count - 1 {
            let mut off = 0;
            while off < 1000 && !TERRAIN_RESYNC.contains(&p.u32(cursor + 12 + off)) {
                off += 1;
            }
            assert!(off < 1000, "Civ6Save: tile resync failed at tile {i}");
            cursor += off;
        }
        let road = p.i16(base + 38) as i64;
        let terrain = d.plot_name(terrain_h, &d.terrains, "KIND_TERRAIN").unwrap_or("UNKNOWN".into());
        plots.push(Plot {
            index: i,
            x: i % width,
            y: i / width,
            terrain,
            feature: d.plot_name(feature_h, &d.features, "KIND_FEATURE"),
            resource: d.plot_name(resource_h, &d.resources, "KIND_RESOURCE"),
            resource_hash: resource_h,
            improvement: d.plot_name(improvement_h, &d.improvements, "KIND_IMPROVEMENT"),
            route_index: if road < 0 { None } else { Some(road & 0xff) },
            pillaged: p.u8(base + 48) & 1 == 1,
            river_map: p.u8(base + 46),
            yield_changes: None,
            owner,
            district_raw,
        });
    }
    Map { width, height: count / width, plots, section_start: start }
}

const YIELDS: [&str; 6] = ["YIELD_FOOD", "YIELD_PRODUCTION", "YIELD_GOLD", "YIELD_SCIENCE", "YIELD_CULTURE", "YIELD_FAITH"];

/// What a flood or an eruption left on each plot: `u32 plotCount, i32 entry[plotCount] (-1 or an index),
/// u32 entryCount, entryCount × { u32 n; n × { yield hash, i32 amount } }`, found by its shape.
fn plot_yield_changes(p: &Bytes, plot_count: i64) -> HashMap<i64, Vec<(String, f64)>> {
    let mut out = HashMap::new();
    let by_hash: HashMap<u32, &str> = YIELDS.iter().map(|y| (civ6_hash(y), *y)).collect();
    let food = civ6_hash("YIELD_FOOD");
    let table = 4 + plot_count * 4;
    let mut at = 0;
    'scan: while at + table + 4 <= p.len() {
        let here = at;
        at += 1;
        if p.u32(here) as i64 != plot_count {
            continue;
        }
        let ec_at = here + table;
        let ec = p.u32(ec_at) as i64;
        if ec > plot_count {
            continue;
        }
        let rec = ec_at + 4;
        if ec > 0 {
            if rec + 8 > p.len() || p.u32(rec) != 6 || p.u32(rec + 4) != food {
                continue;
            }
        }
        let mut index = Vec::with_capacity(plot_count as usize);
        for i in 0..plot_count {
            let v = p.i32(here + 4 + i * 4) as i64;
            if v < -1 || v >= ec.max(1) {
                continue 'scan;
            }
            index.push(v);
        }
        // an all-empty table of the same shape can come first
        if ec == 0 {
            continue;
        }
        let mut entries: Vec<Vec<(String, f64)>> = vec![];
        let mut c = rec;
        for _ in 0..ec {
            let n = p.u32(c) as i64;
            if n == 0 || n > 16 {
                continue 'scan;
            }
            let mut ch = vec![];
            for k in 0..n {
                let Some(name) = by_hash.get(&p.u32(c + 4 + k * 8)) else { continue 'scan };
                let amount = p.i32(c + 8 + k * 8);
                if amount != 0 {
                    ch.push((name.to_string(), amount as f64));
                }
            }
            entries.push(ch);
            c += 4 + n * 8;
        }
        for (plot, e) in index.iter().enumerate() {
            if *e >= 0 {
                if let Some(ch) = entries.get(*e as usize).filter(|c| !c.is_empty()) {
                    out.insert(plot as i64, ch.clone());
                }
            }
        }
        return out;
    }
    out
}

// ---- typed tables --------------------------------------------------------------------------------------------

/// A run of entries, each a type hash of one kind and a fixed-size value (`stride` bytes from hash to hash).
#[derive(Clone)]
pub struct Table {
    pub kind: &'static str,
    pub start: i64,
    pub stride: i64,
    pub count: i64,
    pub names: Vec<&'static str>,
}

const STRIDES_DESC: [i64; 6] = [16, 12, 8, 6, 5, 4];

/// Every run of at least `min` known hashes of one kind at one stride in [from, to): the longest stride first
/// when two explain a run equally, runs never overlapping.
pub fn detect_tables(p: &Bytes, d: &Dict, from: i64, to: i64, min: i64) -> Vec<Table> {
    let from = from.max(0);
    let to = to.min(p.len());
    let type_at = |o: i64| if o + 4 <= to { d.exact(p.u32(o)) } else { None };
    let mut out = vec![];
    let mut off = from;
    while off + 4 <= to {
        let Some(first) = type_at(off) else {
            off += 1;
            continue;
        };
        let mut best: Option<(i64, i64)> = None;
        for stride in STRIDES_DESC {
            let mut count = 1;
            while off + count * stride + 4 <= to {
                match type_at(off + count * stride) {
                    Some(t) if t.kind == first.kind => count += 1,
                    _ => break,
                }
            }
            if count >= min && best.map(|b| count > b.1).unwrap_or(true) {
                best = Some((stride, count));
            }
        }
        let Some((stride, count)) = best else {
            off += 1;
            continue;
        };
        let names = (0..count).map(|i| type_at(off + i * stride).unwrap().name).collect();
        out.push(Table { kind: first.kind, start: off, stride, count, names });
        off += count * stride;
    }
    out
}

/// The value of entry i (the bytes after the hash, up to 4, little-endian).
fn table_int(p: &Bytes, t: &Table, i: i64) -> i64 {
    if i < 0 || i >= t.count {
        return 0;
    }
    let at = t.start + i * t.stride + 4;
    let mut len = t.stride - 4;
    if len == 0 {
        return 0;
    }
    if at + len > p.len() {
        len = p.len() - at;
    }
    match len {
        1 => p.u8(at) as i64,
        2 => p.u16(at) as i64,
        3 => p.u16(at) as i64 | (p.u8(at + 2) as i64) << 16,
        _ => p.u32(at) as i64,
    }
}

fn active(p: &Bytes, t: &Table) -> Vec<(&'static str, i64)> {
    (0..t.count).map(|i| (t.names[i as usize], table_int(p, t, i))).filter(|e| e.1 != 0).collect()
}

fn all_entries(p: &Bytes, t: &Table) -> Vec<(&'static str, i64)> {
    (0..t.count).map(|i| (t.names[i as usize], table_int(p, t, i))).collect()
}

// ---- players -------------------------------------------------------------------------------------------------

#[derive(Clone, Default)]
pub struct PlayerState {
    pub player_id: i64,
    pub techs_researched: Vec<&'static str>,
    pub techs_boosted: Vec<&'static str>,
    pub tech_progress: Vec<(&'static str, f64)>,
    pub current_research: Option<&'static str>,
    pub civics_completed: Vec<&'static str>,
    pub civics_inspired: Vec<&'static str>,
    pub civic_progress: Vec<(&'static str, f64)>,
    pub current_civic: Option<&'static str>,
    pub government: Option<&'static str>,
    pub policies_slotted: Vec<&'static str>,
    pub yields: Vec<(&'static str, f64)>,
    pub units_trained: Vec<(&'static str, f64)>,
    pub gold: f64,
    pub gold_per_turn: f64,
    pub faith: Option<f64>,
    pub pantheon: Option<&'static str>,
    pub era_score: Option<f64>,
    /// the game score at the end of the last turn played (the score shown in the game)
    pub score: Option<f64>,
    pub goody_huts: Vec<(&'static str, f64)>,
    /// the luxuries giving amenities and the city each copy goes to: (city id, resource)
    pub luxuries: Vec<(i64, &'static str)>,
    pub offset: i64,
}

/// The player's luxury amenities: the last table before its techs of a count (1..=256) and that many (city id,
/// resource hash) pairs, a city id being (slot + 1) * 65536 + slot.
fn luxury_table(p: &Bytes, d: &Dict, from: i64, to: i64) -> Vec<(i64, &'static str)> {
    let mut best = vec![];
    let mut q = from.max(0);
    while q + 12 <= to {
        let n = p.i32(q) as i64;
        if (1..=256).contains(&n) && q + 4 + 8 * n <= p.len() {
            let mut out = vec![];
            for k in 0..n {
                let cid = p.i32(q + 4 + 8 * k) as i64;
                let res = d.of_kind(p.u32(q + 8 + 8 * k), "KIND_RESOURCE");
                match res {
                    Some(r) if cid > 0 && (cid >> 16) == (cid & 0xffff) + 1 => out.push((cid, r)),
                    _ => break,
                }
            }
            if out.len() as i64 == n {
                best = out;
                q += 4 + 8 * n;
                continue;
            }
        }
        q += 1;
    }
    best
}

const TECH_COUNT: i64 = 77;
const GOLD_OFFSET: i64 = 1410;
const GOLD_PER_TURN_OFFSET: i64 = 1418;
const RELIGION_SCAN: i64 = 65536;

fn last_before<'t>(all: &'t [Table], index: usize, maxback: usize, f: impl Fn(&Table) -> bool) -> Option<&'t Table> {
    (index.saturating_sub(maxback)..index).rev().map(|i| &all[i]).find(|t| f(t))
}

fn type_at(p: &Bytes, d: &Dict, at: i64, kind: &str) -> Option<&'static str> {
    if at < 0 || at + 4 > p.len() {
        return None;
    }
    d.of_kind(p.u32(at), kind)
}

fn scan_for_type(p: &Bytes, d: &Dict, from: i64, to: i64, kind: &str) -> Option<&'static str> {
    if to >= from {
        (from..=to).find_map(|o| type_at(p, d, o, kind))
    } else {
        (to..=from).rev().find_map(|o| type_at(p, d, o, kind))
    }
}

/// `{POLICY_*, slot}` pairs ending within 64 bytes before `end`, read backwards while they stay policies.
fn slotted_policies(p: &Bytes, d: &Dict, end: i64) -> Vec<&'static str> {
    let mut last = -1;
    let mut o = end - 8;
    while o >= end - 64 && o >= 0 {
        if type_at(p, d, o, "KIND_POLICY").is_some() {
            last = o;
            break;
        }
        o -= 1;
    }
    let mut out = vec![];
    let mut o = last;
    while o >= 0 {
        let Some(policy) = type_at(p, d, o, "KIND_POLICY") else { break };
        let slot = p.i32(o + 4);
        if !(0..=15).contains(&slot) {
            break;
        }
        out.insert(0, policy);
        o -= 8;
    }
    out
}

/// `20, faith×256, 0, BELIEF_* | -1, n, x, y` scanning back from `end`.
fn religion_record(p: &Bytes, d: &Dict, end: i64) -> Option<(f64, Option<&'static str>)> {
    let mut o = end - 4;
    while o >= 12.max(end - RELIGION_SCAN) {
        let belief = p.i32(o);
        if (belief == -1 || type_at(p, d, o, "KIND_BELIEF").is_some()) && p.i32(o - 4) == 0 && p.i32(o - 12) == 20 {
            let faith = p.i32(o - 8);
            if (0..=1_000_000 * 256).contains(&faith) {
                return Some((faith as f64 / 256.0, if belief == -1 { None } else { type_at(p, d, o, "KIND_BELIEF") }));
            }
        }
        o -= 1;
    }
    None
}

fn names_of(p: &Bytes, t: Option<&Table>) -> Vec<&'static str> {
    t.map(|t| active(p, t).into_iter().map(|e| e.0).collect()).unwrap_or_default()
}

fn scaled(p: &Bytes, t: Option<&Table>, s: f64) -> Vec<(&'static str, f64)> {
    let mut out: Vec<(&'static str, f64)> = vec![];
    for (n, v) in t.map(|t| active(p, t)).unwrap_or_default() {
        match out.iter_mut().find(|e| e.0 == n) {
            Some(e) => e.1 = v as f64 / s,
            None => out.push((n, v as f64 / s)),
        }
    }
    out
}

/// Every player object, in file order: anchored on the `TECH s8 n77` progress table after the researched and
/// boosted `TECH s5` tables.
fn parse_players(p: &Bytes, d: &Dict, all: &[Table]) -> Vec<PlayerState> {
    let mut players: Vec<PlayerState> = vec![];
    for i in 0..all.len() {
        let progress = &all[i];
        if progress.kind != "KIND_TECH" || progress.stride != 8 || progress.count != TECH_COUNT || i < 2 {
            continue;
        }
        let (boosted, researched) = (&all[i - 1], &all[i - 2]);
        if researched.kind != "KIND_TECH" || researched.stride != 5 || boosted.kind != "KIND_TECH" || boosted.stride != 5 {
            continue;
        }
        let yields = last_before(all, i - 2, 6, |t| t.kind == "KIND_YIELD" && t.stride == 8);
        let units = last_before(all, i - 2, 8, |t| t.kind == "KIND_UNIT" && t.stride == 8 && t.count > 100);
        let civic_progress = last_before(all, i - 2, 40, |t| t.kind == "KIND_CIVIC" && t.stride == 8);
        let civic_index = civic_progress.map(|c| all.iter().position(|t| std::ptr::eq(t, c)).unwrap() as i64).unwrap_or(-1);
        let inspired = if civic_index > 1 { Some(&all[civic_index as usize - 1]) } else { None };
        let completed = if civic_index > 1 { Some(&all[civic_index as usize - 2]) } else { None };
        let civics_ok = completed.map(|t| t.kind == "KIND_CIVIC" && t.stride == 5).unwrap_or(false)
            && inspired.map(|t| t.kind == "KIND_CIVIC" && t.stride == 5).unwrap_or(false);
        let mut policy_tables: Vec<usize> = vec![];
        let mut slotted_table: Option<usize> = None;
        if civic_index > 1 {
            let mut j = civic_index - 3;
            while j >= 0 && policy_tables.len() < 5 {
                let t = &all[j as usize];
                if t.kind == "KIND_POLICY" && t.stride == 5 {
                    policy_tables.insert(0, j as usize);
                } else if t.kind == "KIND_POLICY" && t.stride == 8 && policy_tables.is_empty() && slotted_table.is_none() {
                    slotted_table = Some(j as usize);
                } else {
                    break;
                }
                j -= 1;
            }
        }
        let ever = if policy_tables.len() == 5 { Some(&all[policy_tables[2]]) } else { None };
        let gov_s8 = policy_tables.first().and_then(|&k| last_before(all, k, 3, |t| t.kind == "KIND_GOVERNMENT" && t.stride == 8));
        let gov_s5 = gov_s8.and_then(|g| {
            let k = all.iter().position(|t| std::ptr::eq(t, g)).unwrap();
            last_before(all, k, 2, |t| t.kind == "KIND_GOVERNMENT" && t.stride == 5)
        });
        let government = gov_s5.or(gov_s8).and_then(|g| scan_for_type(p, d, g.start - 4, g.start - 64, "KIND_GOVERNMENT"));
        let civic_end = civic_progress.map(|c| c.start + c.stride * c.count).unwrap_or(0);
        let promo = last_before(all, i - 2, 40, |t| t.kind == "KIND_PROMOTION_CLASS" && t.stride == 8);
        let religion = promo.and_then(|t| religion_record(p, d, t.start));
        let prev = players.last().map(|x| x.offset).unwrap_or(0);
        let goody = all.iter().find(|t| t.kind == "KIND_GOODY_HUT" && t.stride == 8 && t.start > prev && t.start < researched.start);
        let policies = if let Some(k) = slotted_table {
            all_entries(p, &all[k]).into_iter().filter(|e| (0..=15).contains(&e.1)).map(|e| e.0).collect()
        } else if civics_ok && !names_of(p, ever).is_empty() {
            slotted_policies(p, d, completed.unwrap().start)
        } else {
            vec![]
        };
        players.push(PlayerState {
            player_id: players.len() as i64,
            techs_researched: names_of(p, Some(researched)),
            techs_boosted: names_of(p, Some(boosted)),
            tech_progress: scaled(p, Some(progress), 256.0),
            current_research: type_at(p, d, researched.start - 20, "KIND_TECH"),
            civics_completed: if civics_ok { names_of(p, completed) } else { vec![] },
            civics_inspired: if civics_ok { names_of(p, inspired) } else { vec![] },
            civic_progress: if civics_ok { scaled(p, civic_progress, 256.0) } else { vec![] },
            current_civic: if civics_ok { scan_for_type(p, d, civic_end, civic_end + 16, "KIND_CIVIC") } else { None },
            government,
            policies_slotted: policies,
            yields: scaled(p, yields, 256.0),
            units_trained: scaled(p, units, 1.0),
            gold: p.i32(researched.start + GOLD_OFFSET) as f64 / 256.0,
            gold_per_turn: p.i32(researched.start + GOLD_PER_TURN_OFFSET) as f64 / 256.0,
            faith: religion.map(|r| r.0),
            pantheon: religion.and_then(|r| r.1),
            era_score: None,
            score: None,
            goody_huts: scaled(p, goody, 1.0),
            luxuries: luxury_table(p, d, players.last().map(|x| x.offset).unwrap_or(0).max(researched.start - 40000), researched.start),
            offset: researched.start,
        });
    }
    players
}

/// The player ids of the player objects: the fog-of-war blocks ahead of the map share their order and carry the
/// ids (`6, playerId, revealedCount, u8 0, plotCount, u8[plotCount]`).
fn visibility_ids(p: &Bytes, plot_count: i64, before: i64) -> Vec<i64> {
    let mut out = vec![];
    let mut at = 0;
    while at + 17 + plot_count <= before {
        if p.u32(at) != 6 || p.u8(at + 12) != 0 || p.u32(at + 13) as i64 != plot_count {
            at += 1;
            continue;
        }
        let (pid, count) = (p.u32(at + 4) as i64, p.u32(at + 8) as i64);
        if pid > 63 || count > plot_count {
            at += 1;
            continue;
        }
        let r = at + 17;
        let revealed = p.sub(r, r + plot_count).iter().filter(|&&b| b != 0).count() as i64;
        if revealed != count {
            at += 1;
            continue;
        }
        let mut v = -1;
        let mut o = r + plot_count;
        while o + 12 + 2 * plot_count <= before && o < r + plot_count + 4096 {
            if p.u32(o) as i64 == plot_count && p.u32(o + 8) as i64 == plot_count {
                v = o + 12;
                break;
            }
            o += 1;
        }
        out.push(pid);
        at = if v >= 0 { v + 2 * plot_count } else { r + plot_count };
    }
    out
}

/// Envoys per slot of each player object: 1583 bytes ahead of its first current diplomatic state name, framed by
/// `0x40000000, 0` and `0x05000000` around 63 counts.
/// Also returns, per player, the players it is at war with (its current state with them is a DIPLO_STATE_WAR*).
fn envoys(p: &Bytes, players: &[PlayerState]) -> (Vec<Vec<(i64, i64)>>, Vec<Vec<i64>>) {
    let prefix = b"DIPLO_STATE_";
    let mut out = vec![];
    let mut wars = vec![];
    for i in 0..players.len() {
        let from = players[i].offset;
        let to = players.get(i + 1).map(|x| x.offset).unwrap_or(p.len());
        let mut expected = 0;
        let mut at = p.index_of(prefix, from);
        let mut first = -1;
        let mut war = vec![];
        while at >= 0 && at < to && expected < 64 {
            let len = p.u32(at - 4) as i64;
            let name = p.sub(at, at + len);
            let head = p.u32(at + len);
            let current = head & 0xff == 0 && head & 0xffffff00 != 0;
            let well_formed = name.len() > prefix.len() && name.starts_with(prefix) && name[prefix.len()..].iter().all(|c| c.is_ascii_uppercase() || *c == b'_');
            if well_formed && p.i32(at - 8) as i64 == expected && current {
                if first < 0 {
                    first = at;
                }
                if name.starts_with(b"DIPLO_STATE_WAR") {
                    war.push(expected);
                }
                expected += 1;
            }
            at = p.index_of(prefix, at + 1);
        }
        out.push(if first >= 0 { envoy_counts(p, first) } else { vec![] });
        wars.push(war);
    }
    (out, wars)
}

fn envoy_counts(p: &Bytes, first: i64) -> Vec<(i64, i64)> {
    let start = first - 1583;
    if start < 8 || p.u32(start - 8) != 0x40000000 || p.i32(start - 4) != 0 || p.u32(start + 63 * 4) != 0x05000000 {
        return vec![];
    }
    let mut out = vec![];
    for i in 0..63 {
        let n = p.i32(start + i * 4) as i64;
        if !(0..=200).contains(&n) {
            return vec![];
        }
        if n > 0 {
            out.push((i, n));
        }
    }
    out
}

/// Points banked and earned per turn per great person class: the first two `GREAT_PERSON_CLASS s8` tables of at
/// least 10 entries past the player's last unit record.
fn great_person_points(p: &Bytes, players: &[PlayerState], tables: &[Table], units: &[Unit]) -> Vec<Vec<(&'static str, f64, f64)>> {
    let mut out = vec![];
    for i in 0..players.len() {
        let from = players[i].offset;
        let to = players.get(i + 1).map(|x| x.offset).unwrap_or(p.len());
        let last_unit = units.iter().filter(|u| u.owner == i as i64 && u.offset > from && u.offset < to).map(|u| u.offset + 6400).fold(from, i64::max);
        let mine: Vec<&Table> = tables.iter().filter(|t| t.kind == "KIND_GREAT_PERSON_CLASS" && t.stride == 8 && t.count >= 10 && t.start > last_unit && t.start < to).collect();
        let mut points = vec![];
        if mine.len() >= 2 {
            let (b, r) = (mine[0], mine[1]);
            for k in 0..b.count.min(r.count) {
                let (bi, ri) = (table_int(p, b, k), table_int(p, r, k));
                if bi != 0 || ri != 0 {
                    let name = b.names[k as usize];
                    match points.iter_mut().find(|e: &&mut (&str, f64, f64)| e.0 == name) {
                        Some(e) => *e = (name, bi as f64 / 256.0, ri as f64 / 256.0),
                        None => points.push((name, bi as f64 / 256.0, ri as f64 / 256.0)),
                    }
                }
            }
        }
        out.push(points);
    }
    out
}

/// The recruitment ledger after the last player object: `individual, class, era, cost, recruitedBy, turn`.
pub struct GreatPerson {
    pub individual: &'static str,
    pub class: &'static str,
    pub recruited_by: Option<i64>,
    pub recruited_turn: Option<i64>,
}

fn great_people_ledger(p: &Bytes, d: &Dict, players: &[PlayerState]) -> Vec<GreatPerson> {
    let mut out = vec![];
    let mut o = players.last().map(|x| x.offset).unwrap_or(0);
    while o + 24 <= p.len() {
        if let Some(ind) = d.of_kind(p.u32(o), "KIND_GREAT_PERSON_INDIVIDUAL") {
            if let (Some(cls), Some(_)) = (d.of_kind(p.u32(o + 4), "KIND_GREAT_PERSON_CLASS"), d.of_kind(p.u32(o + 8), "KIND_ERA")) {
                let (by, turn) = (p.i32(o + 16) as i64, p.i32(o + 20) as i64);
                out.push(GreatPerson { individual: ind, class: cls, recruited_by: (by >= 0).then_some(by), recruited_turn: (turn >= 0).then_some(turn) });
                o += 24;
                continue;
            }
        }
        o += 1;
    }
    out
}

pub struct Governor {
    pub player_index: i64,
    pub governor: &'static str,
    pub promotions: Vec<&'static str>,
    pub city_owner_id: Option<i64>,
    pub city_id: Option<i64>,
    pub established: Option<bool>,
}

/// Appointed governors: each `GOVERNOR_*` hash, its `LOC_GOVERNOR_*_NAME` string, `ownerRef, cityId, …` with the
/// establishment flag the tenth int after the name, then a `GOVERNOR_PROMOTION s5` flag table.
fn governors(p: &Bytes, d: &Dict, players: &[PlayerState], tables: &[Table]) -> Vec<Governor> {
    let prefix = b"LOC_GOVERNOR_";
    let mut out = vec![];
    let mut at = p.index_of(prefix, 0);
    while at >= 0 {
        let len = p.u32(at - 4) as i64;
        let name = latin1(p.sub(at, at + len));
        let ty = d.resolve(p.u32(at - 8));
        let shaped = name.len() > 18 && name.starts_with("LOC_GOVERNOR_") && name.ends_with("_NAME")
            && name[13..name.len() - 5].chars().all(|c| c.is_ascii_uppercase() || c == '_');
        if let Some(t) = ty.filter(|t| len < 64 && shaped && t.kind == "KIND_GOVERNOR" && name == format!("LOC_{}_NAME", t.name)) {
            let player_index = players.iter().enumerate().position(|(i, pl)| pl.offset < at && players.get(i + 1).map(|x| x.offset).unwrap_or(p.len()) > at)
                .map(|i| i as i64).unwrap_or(-1);
            let flags = tables.iter().find(|x| x.kind == "KIND_GOVERNOR_PROMOTION" && x.stride == 5 && x.start > at && x.start < at + 2048);
            let after = at + len;
            let city = p.i32(after + 4) as i64;
            out.push(Governor {
                player_index,
                governor: t.name,
                promotions: names_of(p, flags),
                city_owner_id: if city == -1 { None } else { Some(p.i32(after) as i64 & 0xffff) },
                city_id: if city == -1 { None } else { Some(city) },
                established: if city == -1 { None } else { Some(p.i32(after + 36) == 1) },
            });
        }
        at = p.index_of(prefix, at + 1);
    }
    out
}

pub struct Moment {
    pub kind: &'static str,
    pub player_id: i64,
    offset: i64,
}

/// The historic moments: `8, MOMENT_x, turn, x, y, player, index` with consecutive indices.
fn moments(p: &Bytes, d: &Dict) -> Vec<Moment> {
    let mut out: Vec<Moment> = vec![];
    let mut o = 4;
    while o + 28 <= p.len() {
        if p.i32(o - 4) == 8 {
            if let Some(name) = d.of_kind(p.u32(o), "KIND_MOMENT") {
                let (turn, pid, index) = (p.i32(o + 4), p.i32(o + 16) as i64, p.i32(o + 20) as i64);
                if (0..=2000).contains(&turn) && (0..=63).contains(&pid) && index == out.len() as i64 {
                    out.push(Moment { kind: name, player_id: pid, offset: o });
                }
            }
        }
        o += 1;
    }
    out
}

fn commemorations(p: &Bytes, d: &Dict, at: i64) -> Option<(Vec<&'static str>, i64)> {
    let n = p.i32(at) as i64;
    if !(0..=16).contains(&n) || at + 4 + 4 * n > p.len() {
        return None;
    }
    let mut names = vec![];
    for k in 0..n {
        let t = d.resolve(p.u32(at + 4 + 4 * k))?;
        if !t.name.starts_with("COMMEMORATION_") {
            return None;
        }
        names.push(t.name);
    }
    Some((names, at + 4 + 4 * n))
}

/// The dedications taken, per player id: after the moments, `A, 30, nChoices, COMMEMORATION×n, 0, nActive,
/// COMMEMORATION×m` per slot.
fn dedications(p: &Bytes, d: &Dict, from: i64) -> Vec<(i64, Vec<&'static str>)> {
    let mut o = from;
    while o + 16 <= p.len() {
        let Some(choices) = commemorations(p, d, o + 8).filter(|c| !c.0.is_empty()) else {
            o += 1;
            continue;
        };
        let _ = choices;
        let mut out = vec![];
        let marker = p.i32(o + 4);
        let mut at = o;
        for pid in 0..64 {
            if at + 20 > p.len() || p.i32(at + 4) != marker {
                break;
            }
            let Some((_, end)) = commemorations(p, d, at + 8) else { break };
            let Some((act, end2)) = commemorations(p, d, end + 4) else { break };
            out.push((pid, act));
            at = end2;
        }
        return out;
    }
    vec![]
}

/// The great works: after the last player object, `count`, then `6, GREATWORK_x, creator, turn, len, LOC_name`.
fn great_works(p: &Bytes, d: &Dict, from: i64) -> Vec<&'static str> {
    let mut o = from;
    while o + 24 <= p.len() {
        if p.i32(o) == 6 && d.of_kind(p.u32(o + 4), "KIND_GREATWORK").is_some() {
            let count = p.i32(o - 4) as i64;
            if (1..=4096).contains(&count) {
                let mut out = vec![];
                let mut at = o;
                for _ in 0..count {
                    if at + 24 > p.len() || p.i32(at) != 6 {
                        break;
                    }
                    let Some(ty) = d.of_kind(p.u32(at + 4), "KIND_GREATWORK") else { break };
                    let len = p.u32(at + 16) as i64;
                    if len > 128 {
                        break;
                    }
                    out.push(ty);
                    at += 20 + len;
                }
                if out.len() as i64 == count {
                    return out;
                }
            }
        }
        o += 1;
    }
    vec![]
}

/// A city object's great work slots: `count, (BUILDING_x, nSlots, (10, slotType, workIndex | -1) × nSlots) × count`.
fn great_work_slots(p: &Bytes, d: &Dict, from: i64, span: i64) -> Vec<(&'static str, Option<i64>)> {
    let end = p.len().min(from + span);
    let mut o = from;
    while o + 16 <= end {
        let count = p.i32(o) as i64;
        if (1..=40).contains(&count) && d.of_kind(p.u32(o + 4), "KIND_BUILDING").is_some() && p.i32(o + 12) == 10 {
            let mut slots = vec![];
            let mut at = o + 4;
            let mut ok = true;
            'b: for _ in 0..count {
                let b = d.of_kind(p.u32(at), "KIND_BUILDING");
                let n = p.i32(at + 4) as i64;
                let Some(b) = b.filter(|_| (1..=8).contains(&n)) else {
                    ok = false;
                    break;
                };
                at += 8;
                for _ in 0..n {
                    if p.i32(at) != 10 {
                        ok = false;
                        break 'b;
                    }
                    let w = p.i32(at + 8) as i64;
                    slots.push((b, (w >= 0).then_some(w)));
                    at += 12;
                }
            }
            if ok {
                return slots;
            }
        }
        o += 1;
    }
    vec![]
}

/// The last era score of each player from the `REPLAYDATASET_ERASCORE` graph: name, `LOC_HOF_` label, five bytes,
/// kind (1 or 2), count, `(player, turn, value×256)` records.
/// Each player's latest era score and game score (the replay datasets REPLAYDATASET_ERASCORE and
/// REPLAYDATASET_SCOREPERTURN).
fn era_scores(p: &Bytes) -> (HashMap<i64, f64>, HashMap<i64, f64>) {
    let prefix = b"REPLAYDATASET_";
    let mut out = HashMap::new();
    let mut score = HashMap::new();
    let mut at = p.index_of(prefix, 0);
    while at >= 0 {
        let len = p.u32(at - 4) as i64;
        let name = latin1(p.sub(at, at + len));
        let shaped = name.len() > 14 && name[14..].chars().all(|c| c.is_ascii_uppercase() || c == '_');
        // every dataset is read (a malformed one stops the parse, as in the original); the era and game scores are kept
        if len < 64 && shaped && !name.ends_with("_NAME") {
            let series = replay_dataset(p, &name, at - 4);
            if !series.is_empty() && name == "REPLAYDATASET_ERASCORE" {
                out = series;
            } else if !series.is_empty() && name == "REPLAYDATASET_SCOREPERTURN" {
                score = series;
            }
        }
        at = p.index_of(prefix, at + 1);
    }
    (out, score)
}

fn replay_dataset(p: &Bytes, name: &str, from: i64) -> HashMap<i64, f64> {
    let mut out = HashMap::new();
    let mut needle = (name.len() as u32).to_le_bytes().to_vec();
    needle.extend_from_slice(name.as_bytes());
    let i = p.index_of(&needle, from);
    if i < 0 {
        return out;
    }
    let mut q = i + 4 + name.len() as i64;
    let label = p.u32(q) as i64;
    q += 4 + label + 5;
    let kind = p.u32(q);
    if kind != 1 && kind != 2 {
        return out;
    }
    let count = p.u32(q + 4) as i64;
    q += 8;
    let mut k = 0;
    while k < count && q + 12 <= p.len() {
        let (pl, turn) = (p.i32(q) as i64, p.i32(q + 4) as i64);
        if !(0..=63).contains(&pl) || !(0..=10_000).contains(&turn) {
            break;
        }
        out.insert(pl, p.i32(q + 8) as f64 / 256.0);
        k += 1;
        q += 12;
    }
    out
}

// ---- units ---------------------------------------------------------------------------------------------------

pub struct Unit {
    pub type_name: String,
    pub owner: i64,
    pub x: i64,
    pub y: i64,
    pub id: Option<i64>,
    pub builder_charges: Option<f64>,
    pub offset: i64,
}

/// Live unit records: `type, kind (0 for settlers, 1..4, -1 for great people), ?, -9999, -9999, x, y, -1, owner`; builder charges
/// ×256 at +80. Ids come from the per-player unit lists (`[…, 1, index, id, x, y, owner, mp×256, domain, …]`),
/// matched by owner and plot in file order.
fn parse_units(p: &Bytes, d: &Dict, w: i64, h: i64) -> Vec<Unit> {
    let mut units = vec![];
    let end = p.len() - 84;
    let mut i = 0;
    while i < end {
        let here = i;
        i += 1;
        let kind = p.i32(here + 4);
        if !(0..=4).contains(&kind) && kind != -1 {
            continue;
        }
        if p.i32(here + 12) != -9999 || p.i32(here + 16) != -9999 {
            continue;
        }
        let (x, y) = (p.i32(here + 20) as i64, p.i32(here + 24) as i64);
        if x < 0 || y < 0 || x >= w || y >= h || p.i32(here + 28) != -1 {
            continue;
        }
        let owner = p.i32(here + 32) as i64;
        if !(0..=63).contains(&owner) {
            continue;
        }
        let th = p.u32(here);
        let name = d.instance_type(th);
        if kind == -1 && name.is_none() {
            continue;
        }
        let f80 = p.u32(here + 80);
        let charges = (kind == 1 && f80 % 256 == 0 && f80 / 256 <= 10).then(|| (f80 / 256) as f64);
        units.push(Unit { type_name: name.unwrap_or_else(|| format!("UNKNOWN_{th:08x}")), owner, x, y, id: None, builder_charges: charges, offset: here });
    }
    // the per-player unit lists
    let mut list = vec![];
    let lend = p.len() - 60;
    for o in 12..lend.max(12) {
        let domain = p.i32(o + 16);
        if p.i32(o - 8) != 1 || !(0..=3).contains(&domain) {
            continue;
        }
        let (x, y) = (p.i32(o) as i64, p.i32(o + 4) as i64);
        if x < 0 || y < 0 || x >= w || y >= h {
            continue;
        }
        let owner = p.i32(o + 8) as i64;
        if !(0..=63).contains(&owner) {
            continue;
        }
        let mp = p.i32(o + 12);
        if mp < 0 || mp % 256 != 0 || mp > 32 * 256 {
            continue;
        }
        let id = p.i32(o - 4) as i64;
        if id <= 0 || id & 0xffff != p.i32(o - 12) as i64 {
            continue;
        }
        list.push((id, x, y, owner));
    }
    let mut used = vec![false; list.len()];
    for u in units.iter_mut() {
        if let Some(k) = (0..list.len()).find(|&k| !used[k] && list[k].3 == u.owner && list[k].1 == u.x && list[k].2 == u.y) {
            used[k] = true;
            u.id = Some(list[k].0);
        }
    }
    units
}

// ---- cities --------------------------------------------------------------------------------------------------

pub struct City {
    pub id: i64,
    pub slot: i64,
    pub name: String,
    pub player_id: i64,
    pub center: (i64, i64),
    pub capital: bool,
    pub population: i64,
    pub worked: Vec<(i64, i64, i64)>,
    pub current: Option<&'static str>,
    pub progress: Vec<(&'static str, f64)>,
    /// production carried over from a finished item, not yet put into the next (whole units)
    pub held_overflow: Option<f64>,
    pub buildings: Vec<&'static str>,
    pub great_work_slots: Vec<(&'static str, Option<i64>)>,
    owner_index: i64,
    has_id: bool,
}

/// The 37 workable plots of a city in storage order: the centre, then rings 1–3, each starting below the centre.
fn ring_plots(cx: i64, cy: i64) -> Vec<(i64, i64)> {
    const DIRS: [(i64, i64); 6] = [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];
    let (q0, r0) = (cx - (cy - (cy & 1)) / 2, cy);
    let mut out = vec![(cx, cy)];
    for k in 1..=3 {
        let (mut q, mut r) = (q0 + DIRS[5].0 * k, r0 + DIRS[5].1 * k);
        for side in 0..6 {
            let dd = DIRS[(5 + side + 2) % 6];
            for _ in 0..k {
                out.push((q + (r - (r & 1)) / 2, r));
                q += dd.0;
                r += dd.1;
            }
        }
    }
    out
}

/// The city groups of the map tiles, by (owner, slot): the centre is the tile with the lowest district instance.
fn city_groups(map: &Map) -> Vec<(i64, i64, (i64, i64))> {
    let mut groups: Vec<((i64, i64), Vec<usize>)> = vec![];
    let mut index: HashMap<(i64, i64), usize> = HashMap::new();
    for (k, t) in map.plots.iter().enumerate() {
        let Some(o) = &t.owner else { continue };
        let key = (o.player_id, o.city_slot);
        let g = *index.entry(key).or_insert_with(|| {
            groups.push((key, vec![]));
            groups.len() - 1
        });
        groups[g].1.push(k);
    }
    groups.sort_by_key(|g| g.0);
    groups.into_iter().map(|((owner, slot), tiles)| {
        let districts: Vec<usize> = tiles.iter().copied().filter(|&k| map.plots[k].district_raw != 0xffffffff).collect();
        let centre = if !districts.is_empty() {
            districts.iter().copied().reduce(|a, b| if map.plots[a].district_raw & 0xffff <= map.plots[b].district_raw & 0xffff { a } else { b }).unwrap()
        } else {
            tiles.iter().copied().find(|&k| map.plots[k].district_raw == 0x10000).unwrap_or(tiles[0])
        };
        (owner, slot, (map.plots[centre].x, map.plots[centre].y))
    }).collect()
}

const PRODUCTION_KINDS: [&str; 4] = ["KIND_UNIT", "KIND_BUILDING", "KIND_DISTRICT", "KIND_PROJECT"];

/// `id, x, y, owner, originalOwner, owner, -1, id, population` ahead of the city's name.
fn city_header(p: &Bytes, name_offset: i64, owner: i64, index: i64) -> Option<i64> {
    let mut b = name_offset - 36;
    while b >= (name_offset - 8192).max(0) {
        let id = p.i32(b);
        let slot = (id & 0xffff) as i64;
        let hi = ((id as u32) >> 16) as i64;
        if !(slot < index || slot > 0xff || hi <= slot || hi > 0xff) && p.i32(b + 28) == id && p.i32(b + 24) == -1 && p.i32(b + 12) as i64 == owner {
            return Some(b);
        }
        b -= 1;
    }
    None
}

/// The `37, u32×37, 37, u32×37` citizen arrays shortly after the name; both count workers.
fn city_workers(p: &Bytes, name_at: i64, name_len: i64) -> Option<Vec<i64>> {
    let span = 4 + 4 * 37;
    let mut o = name_at + name_len;
    while o + 2 * span <= p.len() && o < name_at + name_len + 256 {
        if p.u32(o) == 37 && p.u32(o + span) == 37 {
            let mut counts: Vec<i64> = (0..37).map(|k| p.u32(o + 4 + 4 * k) as i64).collect();
            if counts.iter().all(|&c| c <= 8) {
                for (k, c) in counts.iter_mut().enumerate() {
                    let extra = p.u32(o + span + 4 + 4 * k as i64) as i64;
                    if extra <= 8 {
                        *c += extra;
                    }
                }
                return Some(counts);
            }
        }
        o += 1;
    }
    None
}

/// The city objects (`LOC_CITY_NAME_*` and ~23 KB after it), joined to the tile groups by owner and slot.
fn parse_cities(p: &Bytes, d: &Dict, map: &Map, players: &[PlayerState], ids: &[i64]) -> Vec<City> {
    let owned: HashMap<i64, (i64, i64)> = map.plots.iter().filter_map(|t| t.owner.as_ref().map(|o| (t.y * map.width + t.x, (o.player_id, o.city_slot)))).collect();
    let mut cities: Vec<City> = city_groups(map).into_iter().map(|(owner, slot, c)| City {
        id: (slot + 1) * 0x10000 + slot, slot, name: String::new(), player_id: owner, center: c, capital: false, population: 0,
        worked: vec![], current: None, progress: vec![], held_overflow: None, buildings: vec![], great_work_slots: vec![], owner_index: slot, has_id: false,
    }).collect();
    let prefix = b"LOC_CITY_NAME_";
    let mut per_owner: HashMap<i64, i64> = HashMap::new();
    let mut from = 0;
    loop {
        let at = p.index_of(prefix, from);
        if at < 0 {
            break;
        }
        from = at + 1;
        if at < 4 {
            continue;
        }
        let len = p.u32(at - 4) as i64;
        if len < prefix.len() as i64 + 1 || len > 64 {
            continue;
        }
        let raw = p.sub(at, at + len);
        if raw.len() as i64 != len || !raw[14..].iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == b'_') {
            continue;
        }
        let s = latin1(raw);
        let name = s[14..].to_string();
        let Some(oi) = players.iter().position(|pl| pl.offset > at) else { continue };
        let owner = ids.get(oi).copied().unwrap_or(oi as i64);
        let idx = *per_owner.get(&owner).unwrap_or(&0);
        per_owner.insert(owner, idx + 1);
        let name_offset = at - 4;
        let end = p.len().min(at + 23500);
        let tables: Vec<Table> = detect_tables(p, d, at, end, 2).into_iter().filter(|t| t.start >= at && t.start < end).collect();
        // the item in production: `16 1 TAG 2 TAG <kind> 0 HASH`
        let mut current = None;
        let mut o = at + 32;
        while o + 12 <= end {
            if let Some(t) = d.resolve(p.u32(o)).filter(|t| PRODUCTION_KINDS.contains(&t.kind)) {
                let k = p.i32(o - 8);
                if p.i32(o - 16) == 2 && p.i32(o - 12) == p.i32(o - 20) && (0..=3).contains(&k) && p.i32(o - 4) == 0 {
                    current = Some(t.name);
                    break;
                }
            }
            o += 1;
        }
        // hammers banked per item ×256; the sparse table (AI weight tables of the same shape are nearly full)
        let mut progress: Vec<(&'static str, f64)> = vec![];
        for t in tables.iter().filter(|t| t.stride == 8 && PRODUCTION_KINDS.contains(&t.kind)) {
            let set: Vec<(&'static str, i64)> = active(p, t).into_iter().filter(|e| e.1 != 0).collect();
            if set.is_empty() || set.len() as i64 > t.count / 4 {
                continue;
            }
            for (n, v) in set {
                if v >= 256 && v < 100_000 * 256 {
                    match progress.iter_mut().find(|e| e.0 == n) {
                        Some(e) => e.1 = v as f64 / 256.0,
                        None => progress.push((n, v as f64 / 256.0)),
                    }
                }
            }
        }
        // the carried-over production: an int32 15 bytes before the last UNIT_SETTLER hash of the city object (after
        // the natural wonder table)
        let settler = civ6_hash("UNIT_SETTLER").to_le_bytes();
        let mut held_overflow = None;
        let mut q = p.index_of(&settler, at);
        while q >= 0 && q < end {
            held_overflow = Some(p.i32(q - 15) as f64);
            q = p.index_of(&settler, q + 1);
        }
        let plot_tables: Vec<&Table> = tables.iter().filter(|t| t.kind == "KIND_BUILDING" && t.stride == 6).collect();
        let header = city_header(p, name_offset, owner, idx);
        let slot = header.map(|h| (p.i32(h) & 0xffff) as i64).unwrap_or(idx);
        let mut buildings: Vec<&'static str> = vec![];
        for t in plot_tables.iter().skip(1) {
            for (n, v) in all_entries(p, t) {
                if v == 65535 || v == 0 {
                    continue;
                }
                if owned.get(&v) != Some(&(owner, slot)) {
                    continue;
                }
                if !buildings.contains(&n) {
                    buildings.push(n);
                }
            }
        }
        let workers = city_workers(p, at, len);
        let centre = header.map(|h| (p.i32(h + 4) as i64, p.i32(h + 8) as i64));
        let worked = match (&workers, centre) {
            (Some(w), Some((cx, cy))) => ring_plots(cx, cy).into_iter().zip(w.iter()).filter(|(_, n)| **n > 0).map(|((x, y), n)| (x, y, *n)).collect(),
            _ => vec![],
        };
        let target = cities.iter().position(|c| c.player_id == owner && c.owner_index == slot)
            .or_else(|| cities.iter().position(|c| c.player_id == owner && c.owner_index == idx));
        let Some(ci) = target else { continue };
        let c = &mut cities[ci];
        c.name = name;
        c.current = current;
        c.progress = progress;
        c.held_overflow = held_overflow;
        c.capital = buildings.contains(&"BUILDING_PALACE");
        c.buildings = buildings;
        c.population = header.map(|h| p.i32(h + 32) as i64).unwrap_or(0);
        c.worked = worked;
        c.great_work_slots = great_work_slots(p, d, name_offset, 23500);
        if let Some(cc) = centre {
            c.center = cc;
        }
        if let Some(h) = header {
            c.id = p.i32(h) as i64;
            c.slot = c.id & 0xffff;
            c.has_id = true;
        }
    }
    let _ = cities.iter().any(|c| c.has_id);
    cities
}

// ---- the save ------------------------------------------------------------------------------------------------

pub struct Player {
    pub state: PlayerState,
    pub civilization: String,
    pub leader: String,
    pub dedications: Option<Vec<&'static str>>,
    pub great_person_points: Vec<(&'static str, f64, f64)>,
    pub envoys: Vec<(i64, i64)>,
    /// the players this player is at war with
    pub at_war: Vec<i64>,
    pub governors: Vec<Governor>,
}

pub struct Save {
    pub turn: i64,
    pub game_speed: String,
    pub map: Map,
    pub players: Vec<Player>,
    pub cities: Vec<City>,
    pub units: Vec<Unit>,
    pub moments: Vec<Moment>,
    pub great_people: Vec<GreatPerson>,
    pub great_works: Vec<&'static str>,
    pub payload: Vec<u8>,
}

/// Reads a .Civ6Save (the whole file).
pub fn parse(file: &[u8], d: &Dict) -> Save {
    let head = header::parse(file, d);
    let payload = decompress(file);
    let p = Bytes(&payload);
    let mut map = parse_map(&p, d);
    for (plot, ch) in plot_yield_changes(&p, map.plots.len() as i64) {
        if let Some(t) = map.plots.get_mut(plot as usize) {
            t.yield_changes = Some(ch);
        }
    }
    let units = parse_units(&p, d, map.width, map.height);
    let tables = detect_tables(&p, d, 0, p.len(), 4);
    let mut states = parse_players(&p, d, &tables);
    let vis = visibility_ids(&p, map.plots.len() as i64, map.section_start);
    let ids: Vec<i64> = if vis.len() == states.len() { vis } else { (0..states.len() as i64).collect() };
    for (s, id) in states.iter_mut().zip(ids.iter()) {
        s.player_id = *id;
    }
    let (env, wars) = envoys(&p, &states);
    let points = great_person_points(&p, &states, &tables, &units);
    let govs = governors(&p, d, &states, &tables);
    let moments = moments(&p, d);
    let ded = dedications(&p, d, moments.last().map(|m| m.offset).unwrap_or(0));
    let tail = states.last().map(|s| s.offset).unwrap_or(0);
    let great_works = great_works(&p, d, tail);
    let (eras, scores) = era_scores(&p);
    for (pl, score) in eras {
        if let Some(s) = states.iter_mut().find(|s| s.player_id == pl) {
            s.era_score = Some(score);
        }
    }
    for (pl, score) in scores {
        if let Some(s) = states.iter_mut().find(|s| s.player_id == pl) {
            s.score = Some(score);
        }
    }
    let great_people = great_people_ledger(&p, d, &states);
    let cities = parse_cities(&p, d, &map, &states, &ids);
    let mut govs_by: HashMap<i64, Vec<Governor>> = HashMap::new();
    for g in govs {
        govs_by.entry(g.player_index).or_default().push(g);
    }
    let players = states.into_iter().enumerate().map(|(index, st)| {
        let slot = head.slots.get(&st.player_id);
        let get = |k: &str| slot.and_then(|s| s.get(k)).and_then(|v| v.as_str().map(String::from));
        Player {
            civilization: get("CIVILIZATION_TYPE_NAME").unwrap_or("UNKNOWN".into()),
            leader: get("LEADER_TYPE_NAME").unwrap_or("UNKNOWN".into()),
            dedications: ded.iter().find(|x| x.0 == st.player_id).map(|x| x.1.clone()),
            great_person_points: points[index].clone(),
            envoys: env[index].clone(),
            at_war: wars[index].clone(),
            governors: govs_by.remove(&(index as i64)).unwrap_or_default(),
            state: st,
        }
    }).collect();
    Save { turn: head.turn, game_speed: head.game_speed, map, players, cities, units, moments, great_people, great_works, payload }
}
