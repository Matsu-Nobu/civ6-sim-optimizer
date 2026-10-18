//! The save header ahead of the compressed payload: the turn, the game speed, and one record per player slot
//! (PLAYER_ID, CIVILIZATION_TYPE_NAME, LEADER_TYPE_NAME, …).
//!
//! The header is a flat list of entries `u32 key, u32 type, …`; the walker (vendored by civ6-savegame-parser from
//! pydt/civ6-save-parser, MIT) splits it into one chunk per entry, and each chunk is decoded as the key/value store
//! `u32 key, u32 type, 8 bytes, value`. A key is civ6_hash of the property's name.
use super::{civ6_hash, index_of, Dict};
use crate::json::Value;
use std::collections::HashMap;

const GAME_TURN: [u8; 4] = [0x9D, 0x2C, 0xE6, 0xBD];
const GAME_SPEED: [u8; 4] = [0x99, 0xB0, 0xD9, 0x05];
const END_UNCOMPRESSED: [u8; 4] = [0, 0, 1, 0];
const COMPRESSED_DATA_END: [u8; 4] = [0, 0, 0xFF, 0xFF];

/// One decoded entry of the walk: its marker and value (strings, numbers, booleans; anything else is None).
struct Entry {
    marker: [u8; 4],
    data: Option<Value>,
}

struct Walker<'a> {
    b: &'a [u8],
    pos: i64,
    next4: [u8; 4],
}

impl<'a> Walker<'a> {
    fn len(&self) -> i64 {
        self.b.len() as i64
    }
    fn byte(&self, o: i64) -> Option<u8> {
        (o >= 0 && o < self.len()).then(|| self.b[o as usize])
    }
    fn u32(&self, o: i64) -> u32 {
        assert!(o >= 0 && o + 4 <= self.len(), "header read at {o} outside the file");
        u32::from_le_bytes(self.b[o as usize..o as usize + 4].try_into().unwrap())
    }
    /// readState: false at the end; else the 4 bytes at pos.
    fn refresh(&mut self) -> bool {
        if self.pos >= self.len() - 4 {
            return false;
        }
        self.next4 = self.b[self.pos as usize..self.pos as usize + 4].try_into().unwrap();
        true
    }
    fn parse_entry(&mut self, dont_skip: bool) -> Entry {
        // the marker is the 4 bytes read when the entry started, also when the type is re-read one byte later
        let marker = self.next4;
        loop {
            let tb_at = self.pos + 4;
            let ty = self.u32(tb_at);
            self.pos += 8;
            if !dont_skip && (u32::from_le_bytes(marker) < 256 || ty == 0) {
                return Entry { marker, data: Some(Value::from("SKIP")) };
            }
            if ty == 0x18 || (self.byte(tb_at) == Some(0x78) && self.byte(tb_at + 1) == Some(0x9C)) {
                self.pos = index_of(self.b, &COMPRESSED_DATA_END, self.pos) + 4;
                return Entry { marker, data: None };
            }
            let data = match ty {
                1 => {
                    self.pos += 8;
                    let v = self.byte(self.pos).unwrap_or(0) != 0;
                    self.pos += 4;
                    Some(Value::from(v))
                }
                2 => {
                    self.pos += 8;
                    let v = self.u32(self.pos);
                    self.pos += 4;
                    Some(Value::from(v as f64))
                }
                0x0A => self.array_0a(),
                3 => {
                    self.pos += 12;
                    None
                }
                0x15 => {
                    let tail = [self.byte(self.pos), self.byte(self.pos + 1), self.byte(self.pos + 2), self.byte(self.pos + 3)];
                    self.pos += if tail == [Some(0), Some(0), Some(0), Some(0x80)] { 20 } else { 12 };
                    None
                }
                4 | 5 => Some(self.string()),
                6 => Some(self.utf_string()),
                0x14 | 0x0D => {
                    self.pos += 16;
                    None
                }
                0x0B => self.array_0b(),
                _ => {
                    self.pos -= 7;
                    continue;
                }
            };
            return Entry { marker, data };
        }
    }
    fn string(&mut self) -> Value {
        let len = (0..3).map(|k| (self.byte(self.pos + k).unwrap_or(0) as i64) << (8 * k)).sum::<i64>();
        self.pos += 2;
        match self.byte(self.pos + 1) {
            Some(0) | Some(0x20) => {
                self.pos += 10;
                Value::from("Don't know what this kind of string is...")
            }
            Some(0x21) => {
                self.pos += 6;
                let nul = index_of(self.b, &[0], self.pos);
                let end = if nul < 0 { self.pos - 1 } else { nul };
                let s = String::from_utf8_lossy(&self.b[self.pos as usize..(end.max(self.pos)) as usize]).into_owned();
                self.pos += len;
                Value::from(s)
            }
            _ => Value::from("Error reading string"),
        }
    }
    fn utf_string(&mut self) -> Value {
        let len = (self.byte(self.pos).unwrap_or(0) as i64 | (self.byte(self.pos + 1).unwrap_or(0) as i64) << 8) * 2;
        self.pos += 2;
        let mark: Vec<Option<u8>> = (0..6).map(|k| self.byte(self.pos + k)).collect();
        if mark == [Some(0), Some(0x21), Some(2), Some(0), Some(0), Some(0)] {
            self.pos += 6;
            let a = self.pos as usize;
            let b = ((self.pos + len - 2).max(self.pos) as usize).min(self.b.len());
            let units: Vec<u16> = self.b[a..b].chunks(2).filter(|c| c.len() == 2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            self.pos += len;
            return Value::from(String::from_utf16_lossy(&units));
        }
        Value::from("Error reading string")
    }
    fn array_0a(&mut self) -> Option<Value> {
        self.pos += 8;
        let n = self.u32(self.pos) as i64;
        self.pos += 4;
        for _ in 0..n {
            let index = self.u32(self.pos) as i64;
            if index > n {
                return Some(Value::from(n as f64));
            }
            assert!(self.refresh(), "header array past the end");
            self.parse_entry(true);
        }
        None
    }
    fn array_0b(&mut self) -> Option<Value> {
        self.pos += 8;
        let n = self.u32(self.pos) as i64;
        self.pos += 4;
        for _ in 0..n {
            if self.byte(self.pos) != Some(0x0A) {
                return Some(Value::from("Error reading array"));
            }
            self.pos += 16;
            loop {
                assert!(self.refresh(), "header array past the end");
                let e = self.parse_entry(false);
                if e.data.as_ref().and_then(|v| v.as_str()) == Some("1") {
                    break;
                }
            }
        }
        None
    }
}

/// The walk: the chunks (one per entry) and the first GAME_TURN and GAME_SPEED values.
fn walk(b: &[u8]) -> (Vec<&[u8]>, Option<Value>, Option<Value>) {
    assert!(b.len() >= 4 && &b[0..4] == b"CIV6", "Not a Civilization 6 save file");
    let mut w = Walker { b, pos: 0, next4: b[0..4].try_into().unwrap() };
    let mut chunks = vec![];
    loop {
        assert!(w.refresh(), "header: no game speed entry");
        if w.next4 == GAME_SPEED {
            break;
        }
        w.pos += 1;
    }
    chunks.push(&b[0..w.pos as usize]);
    let mut chunk_start = w.pos;
    let (mut turn, mut speed) = (None, None);
    let mut ended_inside = false;
    loop {
        if w.next4 == END_UNCOMPRESSED {
            ended_inside = true;
            break;
        }
        let e = w.parse_entry(false);
        if e.marker == GAME_TURN && turn.is_none() {
            turn = Some(e.data.clone().unwrap_or(Value::Null));
        }
        if e.marker == GAME_SPEED && speed.is_none() {
            speed = Some(e.data.clone().unwrap_or(Value::Null));
        }
        let end = w.pos.clamp(chunk_start, b.len() as i64);
        chunks.push(&b[chunk_start as usize..end as usize]);
        chunk_start = w.pos;
        if !w.refresh() {
            break;
        }
    }
    if ended_inside {
        chunks.push(&b[(w.pos as usize).min(b.len())..]);
    }
    (chunks, turn, speed)
}

/// Header property names (their hashes are the keys), and the ones that belong to a player slot.
const NAMES: &[&str] = &["CurrentTurn", "GameSpeed", "MapSize", "NICK_NAME", "AGENDA_TYPE_ID", "AGENDA_TYPE_NAME",
    "CALENDAR_TYPE", "CITY_STATE_COUNT", "CIVILIZATION_ADJECTIVE", "CIVILIZATION_DESCRIPTION",
    "CIVILIZATION_LEVEL_TYPE_ID", "CIVILIZATION_LEVEL_TYPE_NAME", "CIVILIZATION_SHORT_DESCRIPTION",
    "CIVILIZATION_TYPE_ID", "CIVILIZATION_TYPE_NAME", "CONFIGURATION_TYPE", "ENABLED_MODS", "GAMEMODE_APOCALYPSE",
    "GAMEMODE_BARBARIAN_CLANS", "GAMEMODE_DRAMATICAGES", "GAMEMODE_HEROES", "GAMEMODE_MONOPOLIES",
    "GAMEMODE_TOWERDEFENSE", "GAMEMODE_TREE_RANDOMIZER", "GAME_ALLIES_SHARE_VISIBILITY", "GAME_GUID_HI",
    "GAME_GUID_LO", "GAME_HANDICAP", "GAME_MODE", "GAME_NAME", "GAME_NO_BARBARIANS", "GAME_PAUSED", "GAME_REALISM",
    "GAME_ROOT", "GAME_SPEED_NAME", "GAME_START_ERA", "GAME_START_ERA_NAME", "GAME_START_TURN", "GAME_START_YEAR",
    "GAME_STATE", "GAME_SYNC_RANDOM_SEED", "GAME_TURN_LIMIT", "HANDICAP_TYPE_ID", "HOTSEAT_PASSWORD", "IS_ALIVE",
    "IS_LOCKED", "LEADER_DOMAIN", "LEADER_NAME", "LEADER_TYPE_ID", "LEADER_TYPE_NAME", "LEADER_VALUES",
    "LEADER_VALUE_DOMAIN", "MAP_PINS", "MOD_ID", "MOD_READY_STATUS", "MOD_SUBSCRIPTION_ID", "MOD_TITLE_KEY",
    "MOD_VERSION", "NETWORK_NAME", "NO_DUPLICATE_CIVILIZATIONS", "NO_DUPLICATE_LEADERS", "NO_TEAMS",
    "PLAYER_COLOR_ALTERNATE", "PLAYER_COLOR_TYPE", "PLAYER_ID", "READY_STATUS", "RULESET", "RULESET_NAME",
    "SAVED_GAME", "SLOT_STATUS", "TEAM", "TURN_TIMER_TIME", "TURN_TIMER", "VICTORY_TECHNOLOGY", "VICTORY_CONQUEST",
    "VICTORY_RELIGIOUS", "VICTORY_CULTURE", "VICTORY_SCORE", "VICTORY_DIPLOMATIC",
    "s Lua/SQL/XML (33 hits on dip-a-1)
    ", ", ", ", ", ", ", ", ", ", ", ", ", ",
    ", ", ", ", ", ", ", ", ",
    ", ", ",
    ", ", ", ", ", ", ", ", ", ", ", ", ", ", ", ", ", ",
    ", ", ", ", ", ", ", ", ", ", ", ", ", ",
    ", ", ", ",
    ", ", "];
const PLAYER_KEYS: [&str; 29] = ["PLAYER_ID", "NICK_NAME", "AGENDA_TYPE_ID", "AGENDA_TYPE_NAME", "CIVILIZATION_ADJECTIVE", "CIVILIZATION_DESCRIPTION",
    "CIVILIZATION_LEVEL_TYPE_ID", "CIVILIZATION_LEVEL_TYPE_NAME", "CIVILIZATION_SHORT_DESCRIPTION", "CIVILIZATION_TYPE_ID", "CIVILIZATION_TYPE_NAME",
    "CONFIGURATION_TYPE", "HANDICAP_TYPE_ID", "HOTSEAT_PASSWORD", "IS_ALIVE", "IS_LOCKED", "LEADER_DOMAIN", "LEADER_NAME", "LEADER_TYPE_ID",
    "LEADER_TYPE_NAME", "LEADER_VALUES", "LEADER_VALUE_DOMAIN", "PLAYER_COLOR_ALTERNATE", "PLAYER_COLOR_TYPE", "READY_STATUS", "SLOT_STATUS", "TEAM",
    "MOD_READY_STATUS", "NETWORK_NAME"];

/// One entry of the key/value store: `u32 key, u32 type, 8 bytes, value` (1 bool, 2 int, 3 type hash, 4/5 string
/// `u24 len | 0x21 << 24, u32 1, bytes` right after the type).
fn read_entry(c: &[u8], d: &Dict) -> Option<(u32, Value)> {
    if c.len() < 16 {
        return None;
    }
    let u = |o: usize| u32::from_le_bytes(c[o..o + 4].try_into().unwrap());
    let (key, ty) = (u(0), u(4));
    let v = match ty {
        1 => Value::from(c[16] == 1),
        2 => Value::from(u(16) as i32 as f64),
        3 => {
            let h = u(16);
            d.resolve(h).map(|t| Value::from(t.name)).unwrap_or(Value::from(h as f64))
        }
        4 | 5 => {
            let len = (c[8] as usize) | (c[9] as usize) << 8 | (c[10] as usize) << 16;
            if c[11] != 0x21 || 16 + len > c.len() {
                return None;
            }
            Value::from(String::from_utf8_lossy(&c[16..16 + len.saturating_sub(1)]).into_owned())
        }
        _ => return None,
    };
    Some((key, v))
}

pub struct Header {
    pub turn: i64,
    pub game_speed: String,
    /// The fuller record of each player id (the header lists every slot twice).
    pub slots: HashMap<i64, HashMap<String, Value>>,
}

pub fn parse(file: &[u8], d: &Dict) -> Header {
    let (chunks, turn, speed) = walk(file);
    let dict: HashMap<u32, &str> = NAMES.iter().map(|n| (civ6_hash(n), *n)).collect();
    let mut players: Vec<HashMap<String, Value>> = vec![];
    let mut game: HashMap<String, Value> = HashMap::new();
    let mut in_player = false;
    for c in chunks {
        let Some((key, value)) = read_entry(c, d) else { continue };
        let name = dict.get(&key).copied();
        let label = name.map(String::from).unwrap_or(format!("{key:08x}"));
        if name == Some("PLAYER_ID") {
            players.push(HashMap::new());
            in_player = true;
        }
        let to_game = name.map(|n| !PLAYER_KEYS.contains(&n)).unwrap_or(false) || !in_player;
        let target = if to_game { &mut game } else { players.last_mut().unwrap() };
        target.entry(label).or_insert(value);
    }
    let mut slots: HashMap<i64, HashMap<String, Value>> = HashMap::new();
    for s in players {
        let Some(id) = s.get("PLAYER_ID").and_then(|v| v.as_i64()) else { continue };
        if slots.get(&id).map(|o| s.len() > o.len()).unwrap_or(true) {
            slots.insert(id, s);
        }
    }
    let turn = turn.and_then(|v| v.as_f64()).filter(|t| *t != 0.0).unwrap_or(0.0) as i64;
    let game_speed = speed.and_then(|v| v.as_str().map(String::from)).unwrap_or("UNKNOWN".into());
    Header { turn, game_speed, slots }
}
