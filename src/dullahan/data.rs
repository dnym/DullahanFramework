//! Git-friendly data-driven layer:
//! - `Pool<T>` + `Handle<T>`: flat generational arena (O(1) insert/remove/lookup, no ECS overhead)
//! - `.cfg`: flat `key = value` tuning config with live hot-reload & runtime `dctl set` overrides
//! - `.tbl`: line-oriented typed records (`@cols ...`)
//! - `.lvl`: declarative spatial placement lines (`kind name x y z key=val ...`)
//! - `Snapshot`: schema-tolerant plaintext state serializer for Tier 3 sub-second restarts

use crate::dullahan::math::{Color, Vec3};
use std::fs;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

// ---------------------------------------------------------------------------
// Pool<T> + Handle<T> (Generational Arena)
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct Handle<T> {
    pub index: u32,
    pub generation: u32,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Copy for Handle<T> {}
impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}
impl<T> Eq for Handle<T> {}

struct PoolSlot<T> {
    generation: u32,
    value: Option<T>,
}

pub struct Pool<T> {
    slots: Vec<PoolSlot<T>>,
    free: Vec<u32>,
    count: usize,
}

impl<T> Default for Pool<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Pool<T> {
    pub fn new() -> Self {
        Self { slots: Vec::new(), free: Vec::new(), count: 0 }
    }

    pub fn insert(&mut self, value: T) -> Handle<T> {
        self.count += 1;
        if let Some(idx) = self.free.pop() {
            let slot = &mut self.slots[idx as usize];
            slot.value = Some(value);
            Handle { index: idx, generation: slot.generation, _marker: PhantomData }
        } else {
            let idx = self.slots.len() as u32;
            self.slots.push(PoolSlot { generation: 1, value: Some(value) });
            Handle { index: idx, generation: 1, _marker: PhantomData }
        }
    }

    pub fn remove(&mut self, h: Handle<T>) -> Option<T> {
        let slot = self.slots.get_mut(h.index as usize)?;
        if slot.generation != h.generation {
            return None;
        }
        let val = slot.value.take()?;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        self.free.push(h.index);
        self.count -= 1;
        Some(val)
    }

    pub fn get(&self, h: Handle<T>) -> Option<&T> {
        let slot = self.slots.get(h.index as usize)?;
        if slot.generation == h.generation { slot.value.as_ref() } else { None }
    }

    pub fn get_mut(&mut self, h: Handle<T>) -> Option<&mut T> {
        let slot = self.slots.get_mut(h.index as usize)?;
        if slot.generation == h.generation { slot.value.as_mut() } else { None }
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn clear(&mut self) {
        self.slots.clear();
        self.free.clear();
        self.count = 0;
    }

    pub fn iter(&self) -> impl Iterator<Item = (Handle<T>, &T)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| {
            s.value.as_ref().map(|v| {
                (Handle { index: i as u32, generation: s.generation, _marker: PhantomData }, v)
            })
        })
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Handle<T>, &mut T)> {
        self.slots.iter_mut().enumerate().filter_map(|(i, s)| {
            let g = s.generation;
            s.value.as_mut().map(|v| {
                (Handle { index: i as u32, generation: g, _marker: PhantomData }, v)
            })
        })
    }
}

// ---------------------------------------------------------------------------
// .cfg Parser (Flat key = value)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct Config {
    entries: Vec<(String, String)>,
    overrides: Vec<(String, String)>,
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut entries = Vec::new();
        for (line_idx, raw) in text.lines().enumerate() {
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                return Err(format!("line {}: expected 'key = value'", line_idx + 1));
            };
            let key = k.trim().to_string();
            let val = v.trim().to_string();
            if key.is_empty() {
                return Err(format!("line {}: empty key", line_idx + 1));
            }
            upsert(&mut entries, key, val);
        }
        Ok(Self { entries, overrides: Vec::new() })
    }

    pub fn set_override(&mut self, key: &str, val: &str) {
        upsert(&mut self.overrides, key.to_string(), val.to_string());
    }

    pub fn clear_overrides(&mut self) {
        self.overrides.clear();
    }

    pub fn raw(&self, key: &str) -> Option<&str> {
        self.overrides
            .iter()
            .find(|(k, _)| k == key)
            .or_else(|| self.entries.iter().find(|(k, _)| k == key))
            .map(|(_, v)| v.as_str())
    }

    pub fn f32(&self, key: &str, default: f32) -> f32 {
        self.raw(key).and_then(|s| s.parse::<f32>().ok()).unwrap_or(default)
    }

    pub fn i32(&self, key: &str, default: i32) -> i32 {
        self.raw(key).and_then(|s| s.parse::<i32>().ok()).unwrap_or(default)
    }

    pub fn bool(&self, key: &str, default: bool) -> bool {
        self.raw(key).and_then(parse_bool).unwrap_or(default)
    }

    pub fn str<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.raw(key).unwrap_or(default)
    }

    pub fn vec3(&self, key: &str, default: Vec3) -> Vec3 {
        self.raw(key).and_then(parse_vec3).unwrap_or(default)
    }

    pub fn color(&self, key: &str, default: Color) -> Color {
        self.raw(key).and_then(parse_hex_color).unwrap_or(default)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

// ---------------------------------------------------------------------------
// .tbl Parser (Line-oriented typed table)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct TableRow {
    fields: Vec<(String, String)>,
}

impl TableRow {
    pub fn raw(&self, col: &str) -> Option<&str> {
        self.fields.iter().find(|(k, _)| k == col).map(|(_, v)| v.as_str())
    }

    pub fn str<'a>(&'a self, col: &str, default: &'a str) -> &'a str {
        self.raw(col).unwrap_or(default)
    }

    pub fn f32(&self, col: &str, default: f32) -> f32 {
        self.raw(col).and_then(|s| s.parse::<f32>().ok()).unwrap_or(default)
    }

    pub fn i32(&self, col: &str, default: i32) -> i32 {
        self.raw(col).and_then(|s| s.parse::<i32>().ok()).unwrap_or(default)
    }

    pub fn bool(&self, col: &str, default: bool) -> bool {
        self.raw(col).and_then(parse_bool).unwrap_or(default)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<TableRow>,
}

impl Table {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut columns: Vec<String> = Vec::new();
        let mut rows: Vec<TableRow> = Vec::new();

        for (line_idx, raw) in text.lines().enumerate() {
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("@cols") {
                columns = rest.split_whitespace().map(|s| s.to_string()).collect();
                if columns.is_empty() {
                    return Err(format!("line {}: @cols requires at least one column", line_idx + 1));
                }
                continue;
            }
            if columns.is_empty() {
                return Err(format!("line {}: data row before @cols header", line_idx + 1));
            }
            let vals = split_tokens(line);
            if vals.len() != columns.len() {
                return Err(format!(
                    "line {}: expected {} columns, found {}",
                    line_idx + 1,
                    columns.len(),
                    vals.len()
                ));
            }
            let fields = columns.iter().cloned().zip(vals.into_iter()).collect();
            rows.push(TableRow { fields });
        }
        Ok(Self { columns, rows })
    }

    pub fn by_id(&self, id: &str) -> Option<&TableRow> {
        self.rows.iter().find(|r| r.raw("id") == Some(id))
    }
}

// ---------------------------------------------------------------------------
// .lvl Parser (Declarative spatial placement)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct LevelEntry {
    pub kind: String,
    pub name: String,
    pub pos: Vec3,
    pub has_pos: bool,
    pub attrs: Vec<(String, String)>,
}

impl LevelEntry {
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn attr_f32(&self, key: &str, default: f32) -> f32 {
        self.attr(key).and_then(|s| s.parse::<f32>().ok()).unwrap_or(default)
    }

    pub fn attr_vec3(&self, key: &str, default: Vec3) -> Vec3 {
        self.attr(key).and_then(parse_vec3).unwrap_or(default)
    }

    pub fn attr_color(&self, key: &str, default: Color) -> Color {
        self.attr(key).and_then(parse_hex_color).unwrap_or(default)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Level {
    pub entries: Vec<LevelEntry>,
}

impl Level {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut entries = Vec::new();
        for (line_idx, raw) in text.lines().enumerate() {
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            let toks = split_tokens(line);
            if toks.len() < 2 {
                return Err(format!("line {}: expected '<kind> <name> [x y z] [k=v...]'", line_idx + 1));
            }
            let kind = toks[0].clone();
            let name = toks[1].clone();
            let mut idx = 2;
            let mut pos = Vec3::ZERO;
            let mut has_pos = false;

            if idx + 2 < toks.len()
                && !toks[idx].contains('=')
                && let (Ok(x), Ok(y), Ok(z)) = (
                    toks[idx].parse::<f32>(),
                    toks[idx + 1].parse::<f32>(),
                    toks[idx + 2].parse::<f32>(),
                )
            {
                pos = Vec3::new(x, y, z);
                has_pos = true;
                idx += 3;
            }

            let mut attrs = Vec::new();
            for tok in &toks[idx..] {
                let Some((k, v)) = tok.split_once('=') else {
                    return Err(format!("line {}: expected key=value, got '{tok}'", line_idx + 1));
                };
                attrs.push((k.to_string(), v.to_string()));
            }
            entries.push(LevelEntry { kind, name, pos, has_pos, attrs });
        }
        Ok(Self { entries })
    }
}

// ---------------------------------------------------------------------------
// Schema-Tolerant Plaintext Snapshot (Tier 3 Fast-Restart)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    sections: Vec<(String, Vec<(String, String)>)>,
}

impl Snapshot {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_str(&mut self, section: &str, key: &str, val: &str) {
        if let Some((_, entries)) = self.sections.iter_mut().find(|(s, _)| s == section) {
            upsert(entries, key.to_string(), val.to_string());
        } else {
            self.sections.push((section.to_string(), vec![(key.to_string(), val.to_string())]));
        }
    }

    pub fn set_f32(&mut self, section: &str, key: &str, val: f32) {
        self.set_str(section, key, &format!("{val:.6}"));
    }

    pub fn set_i32(&mut self, section: &str, key: &str, val: i32) {
        self.set_str(section, key, &val.to_string());
    }

    pub fn set_bool(&mut self, section: &str, key: &str, val: bool) {
        self.set_str(section, key, if val { "1" } else { "0" });
    }

    pub fn set_vec3(&mut self, section: &str, key: &str, v: Vec3) {
        self.set_str(section, key, &format!("{:.6},{:.6},{:.6}", v.x, v.y, v.z));
    }

    pub fn get_str<'a>(&'a self, section: &str, key: &str, default: &'a str) -> &'a str {
        self.sections
            .iter()
            .find(|(s, _)| s == section)
            .and_then(|(_, entries)| entries.iter().find(|(k, _)| k == key))
            .map(|(_, v)| v.as_str())
            .unwrap_or(default)
    }

    pub fn get_f32(&self, section: &str, key: &str, default: f32) -> f32 {
        self.get_str(section, key, "").parse::<f32>().unwrap_or(default)
    }

    pub fn get_i32(&self, section: &str, key: &str, default: i32) -> i32 {
        self.get_str(section, key, "").parse::<i32>().unwrap_or(default)
    }

    pub fn get_bool(&self, section: &str, key: &str, default: bool) -> bool {
        parse_bool(self.get_str(section, key, "")).unwrap_or(default)
    }

    pub fn get_vec3(&self, section: &str, key: &str, default: Vec3) -> Vec3 {
        parse_vec3(self.get_str(section, key, "")).unwrap_or(default)
    }

    pub fn to_text(&self) -> String {
        let mut out = String::from("# Dullahan Snapshot v1\n");
        for (sec, entries) in &self.sections {
            out.push_str(&format!("\n[{sec}]\n"));
            for (k, v) in entries {
                out.push_str(&format!("{k} = {v}\n"));
            }
        }
        out
    }

    pub fn from_text(text: &str) -> Self {
        let mut snap = Self::new();
        let mut cur_sec = "root".to_string();
        for raw in text.lines() {
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') && line.len() >= 3 {
                cur_sec = line[1..line.len() - 1].trim().to_string();
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                snap.set_str(&cur_sec, k.trim(), v.trim());
            }
        }
        snap
    }

    pub fn save_file<P: AsRef<Path>>(&self, path: P) -> Result<(), String> {
        fs::write(path.as_ref(), self.to_text()).map_err(|e| format!("{}: {e}", path.as_ref().display()))
    }

    pub fn load_file<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let text = fs::read_to_string(path.as_ref()).map_err(|e| format!("{}: {e}", path.as_ref().display()))?;
        Ok(Self::from_text(&text))
    }
}

// ---------------------------------------------------------------------------
// DataStore — Hot-Reloading Manager for Tier 2 (.cfg / .tbl / .lvl)
// ---------------------------------------------------------------------------

struct WatchedFile<T> {
    name: String,
    path: PathBuf,
    mtime: Option<SystemTime>,
    data: T,
}

pub struct DataStore {
    root: PathBuf,
    configs: Vec<WatchedFile<Config>>,
    tables: Vec<WatchedFile<Table>>,
    levels: Vec<WatchedFile<Level>>,
    last_poll: Instant,
    poll_interval: Duration,
    /// Incremented whenever any `.cfg`, `.tbl`, or `.lvl` reloads or receives a `dctl set`.
    pub generation: u64,
    pub total_reloads: u32,
}

impl DataStore {
    pub fn new<P: AsRef<Path>>(data_root: P) -> Self {
        Self {
            root: data_root.as_ref().to_path_buf(),
            configs: Vec::new(),
            tables: Vec::new(),
            levels: Vec::new(),
            last_poll: Instant::now(),
            poll_interval: Duration::from_millis(120),
            generation: 1,
            total_reloads: 0,
        }
    }

    fn resolve(&self, name: &str) -> PathBuf {
        let p = Path::new(name);
        if p.is_absolute() || p.exists() {
            p.to_path_buf()
        } else {
            self.root.join(name)
        }
    }

    pub fn load_config(&mut self, name: &str) -> &Config {
        if let Some(i) = self.configs.iter().position(|c| c.name == name) {
            return &self.configs[i].data;
        }
        let path = self.resolve(name);
        let mtime = file_mtime(&path);
        let data = match fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| Config::parse(&t)) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("[data] ERROR loading {}: {e}", path.display());
                Config::default()
            }
        };
        self.configs.push(WatchedFile { name: name.to_string(), path, mtime, data });
        &self.configs.last().unwrap().data
    }

    pub fn load_table(&mut self, name: &str) -> &Table {
        if let Some(i) = self.tables.iter().position(|t| t.name == name) {
            return &self.tables[i].data;
        }
        let path = self.resolve(name);
        let mtime = file_mtime(&path);
        let data = match fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| Table::parse(&t)) {
            Ok(tbl) => tbl,
            Err(e) => {
                eprintln!("[data] ERROR loading {}: {e}", path.display());
                Table::default()
            }
        };
        self.tables.push(WatchedFile { name: name.to_string(), path, mtime, data });
        &self.tables.last().unwrap().data
    }

    pub fn load_level(&mut self, name: &str) -> &Level {
        if let Some(i) = self.levels.iter().position(|l| l.name == name) {
            return &self.levels[i].data;
        }
        let path = self.resolve(name);
        let mtime = file_mtime(&path);
        let data = match fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| Level::parse(&t)) {
            Ok(lvl) => lvl,
            Err(e) => {
                eprintln!("[data] ERROR loading {}: {e}", path.display());
                Level::default()
            }
        };
        self.levels.push(WatchedFile { name: name.to_string(), path, mtime, data });
        &self.levels.last().unwrap().data
    }

    pub fn config(&self, name: &str) -> Option<&Config> {
        self.configs.iter().find(|c| c.name == name).map(|c| &c.data)
    }

    pub fn table(&self, name: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.name == name).map(|t| &t.data)
    }

    pub fn level(&self, name: &str) -> Option<&Level> {
        self.levels.iter().find(|l| l.name == name).map(|l| &l.data)
    }

    /// Sets a live runtime override on all loaded configs (or creates a `"live"` config if none loaded).
    pub fn set_override(&mut self, key: &str, val: &str) {
        if self.configs.is_empty() {
            let mut cfg = Config::default();
            cfg.set_override(key, val);
            self.configs.push(WatchedFile {
                name: "live".into(),
                path: PathBuf::new(),
                mtime: None,
                data: cfg,
            });
        } else {
            for c in &mut self.configs {
                c.data.set_override(key, val);
            }
        }
        self.generation += 1;
    }

    /// Polls watched `.cfg`, `.tbl`, and `.lvl` files; returns `(reloaded, errors)`.
    pub fn hot_reload(&mut self, force: bool) -> (u32, u32) {
        if !force && self.last_poll.elapsed() < self.poll_interval {
            return (0, 0);
        }
        self.last_poll = Instant::now();
        let mut reloaded = 0u32;
        let mut errors = 0u32;

        for c in &mut self.configs {
            if c.path.as_os_str().is_empty() {
                continue;
            }
            let cur = file_mtime(&c.path);
            if cur.is_some() && cur != c.mtime {
                c.mtime = cur;
                match fs::read_to_string(&c.path).map_err(|e| e.to_string()).and_then(|t| Config::parse(&t)) {
                    Ok(mut parsed) => {
                        parsed.overrides = c.data.overrides.clone();
                        c.data = parsed;
                        reloaded += 1;
                        self.total_reloads += 1;
                        self.generation += 1;
                        eprintln!("[data] reloaded {}", c.path.display());
                    }
                    Err(e) => {
                        errors += 1;
                        eprintln!("[data] ERROR reloading {}: {e}", c.path.display());
                    }
                }
            }
        }

        for t in &mut self.tables {
            let cur = file_mtime(&t.path);
            if cur.is_some() && cur != t.mtime {
                t.mtime = cur;
                match fs::read_to_string(&t.path).map_err(|e| e.to_string()).and_then(|txt| Table::parse(&txt)) {
                    Ok(parsed) => {
                        t.data = parsed;
                        reloaded += 1;
                        self.total_reloads += 1;
                        self.generation += 1;
                        eprintln!("[data] reloaded {}", t.path.display());
                    }
                    Err(e) => {
                        errors += 1;
                        eprintln!("[data] ERROR reloading {}: {e}", t.path.display());
                    }
                }
            }
        }

        for l in &mut self.levels {
            let cur = file_mtime(&l.path);
            if cur.is_some() && cur != l.mtime {
                l.mtime = cur;
                match fs::read_to_string(&l.path).map_err(|e| e.to_string()).and_then(|txt| Level::parse(&txt)) {
                    Ok(parsed) => {
                        l.data = parsed;
                        reloaded += 1;
                        self.total_reloads += 1;
                        self.generation += 1;
                        eprintln!("[data] reloaded {}", l.path.display());
                    }
                    Err(e) => {
                        errors += 1;
                        eprintln!("[data] ERROR reloading {}: {e}", l.path.display());
                    }
                }
            }
        }

        (reloaded, errors)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn file_mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

fn upsert(list: &mut Vec<(String, String)>, key: String, val: String) {
    if let Some(entry) = list.iter_mut().find(|(k, _)| *k == key) {
        entry.1 = val;
    } else {
        list.push((key, val));
    }
}

fn strip_comment(line: &str) -> &str {
    // Preserve `#RRGGBB` hex colors while stripping `# comment` when preceded by start-of-line or whitespace.
    let bytes = line.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'#' {
            let prev_ws = i == 0 || bytes[i - 1].is_ascii_whitespace();
            let next_hex = bytes.get(i + 1).is_some_and(|b| b.is_ascii_hexdigit());
            if prev_ws && !next_hex {
                return &line[..i];
            }
        }
    }
    line
}

fn split_tokens(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for ch in line.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            c if c.is_whitespace() && !in_quotes => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn parse_vec3(s: &str) -> Option<Vec3> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 3 {
        return None;
    }
    Some(Vec3::new(
        parts[0].trim().parse().ok()?,
        parts[1].trim().parse().ok()?,
        parts[2].trim().parse().ok()?,
    ))
}

fn parse_hex_color(s: &str) -> Option<Color> {
    let hex = s.trim().trim_start_matches('#');
    if hex.len() == 6 {
        let v = u32::from_str_radix(hex, 16).ok()?;
        Some(Color::from_hex(v))
    } else if hex.len() == 8 {
        let v = u32::from_str_radix(hex, 16).ok()?;
        let r = ((v >> 24) & 0xFF) as f32 / 255.0;
        let g = ((v >> 16) & 0xFF) as f32 / 255.0;
        let b = ((v >> 8) & 0xFF) as f32 / 255.0;
        let a = (v & 0xFF) as f32 / 255.0;
        Some(Color::rgba(r, g, b, a))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_generational_handles_prevent_stale_access() {
        let mut pool = Pool::new();
        let a = pool.insert(10);
        let b = pool.insert(20);
        assert_eq!(pool.get(a), Some(&10));
        assert_eq!(pool.remove(a), Some(10));
        let c = pool.insert(30); // reuses slot 0 with bumped generation
        assert_eq!(pool.get(a), None);
        assert_eq!(pool.get(b), Some(&20));
        assert_eq!(pool.get(c), Some(&30));
        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn parses_cfg_tbl_lvl_and_schema_tolerant_snapshot() {
        let mut cfg = Config::parse(
            "# comment\nplayer.speed = 9.5 # inline\nlight.color = #FFAA44\nspawn.pos = 1, 2, 3\n",
        )
        .unwrap();
        assert!((cfg.f32("player.speed", 0.0) - 9.5).abs() < 1e-5);
        assert_eq!(cfg.vec3("spawn.pos", Vec3::ZERO), Vec3::new(1.0, 2.0, 3.0));
        assert!((cfg.color("light.color", Color::BLACK).r - 1.0).abs() < 1e-3);
        cfg.set_override("player.speed", "15.0");
        assert!((cfg.f32("player.speed", 0.0) - 15.0).abs() < 1e-5);

        let tbl = Table::parse(
            "@cols id damage mesh\nsword 25 weapons/sword.glb\naxe 40 weapons/axe.glb\n",
        )
        .unwrap();
        assert_eq!(tbl.by_id("axe").unwrap().i32("damage", 0), 40);

        let lvl = Level::parse(
            "prop crate 4.0 0.0 -2.5 yaw=45 scale=1.2\nlight porch 0 3 1 color=#ffaa44 radius=8\n",
        )
        .unwrap();
        assert_eq!(lvl.entries.len(), 2);
        assert_eq!(lvl.entries[0].pos, Vec3::new(4.0, 0.0, -2.5));
        assert!((lvl.entries[0].attr_f32("yaw", 0.0) - 45.0).abs() < 1e-5);

        let mut snap = Snapshot::new();
        snap.set_vec3("player", "pos", Vec3::new(3.0, 1.5, -2.0));
        snap.set_i32("player", "old_removed_field", 99);
        let restored = Snapshot::from_text(&snap.to_text());
        assert_eq!(restored.get_vec3("player", "pos", Vec3::ZERO), Vec3::new(3.0, 1.5, -2.0));
        assert_eq!(restored.get_f32("player", "brand_new_field", 42.0), 42.0);
    }
}
