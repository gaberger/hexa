//! The book as a version 1 JSON file (ADR-2610281300 §2–§5).

use std::fmt;
use std::io::Write;
use std::path::PathBuf;

use serde::de::{Deserializer, MapAccess, Visitor};
use serde::Deserialize;

use crate::ports::{DateText, HabitBook, HabitName, HabitStore, StoreError};

pub struct JsonFileStore<'a> {
    path: PathBuf,
    is_default: bool,
    dates: &'a dyn DateText,
}

impl<'a> JsonFileStore<'a> {
    pub fn new(path: PathBuf, is_default: bool, dates: &'a dyn DateText) -> Self {
        JsonFileStore { path, is_default, dates }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileV1 {
    #[allow(dead_code)]
    version: u64,
    habits: Entries,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    checkins: Vec<String>,
}

/// The habits, in file order, refusing a key written twice.
struct Entries(Vec<(String, Entry)>);

impl<'de> Deserialize<'de> for Entries {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Entries;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an object of habits")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Entries, A::Error> {
                let mut out: Vec<(String, Entry)> = Vec::new();
                while let Some((k, v)) = map.next_entry::<String, Entry>()? {
                    if out.iter().any(|(e, _)| *e == k) {
                        return Err(serde::de::Error::custom(format!("duplicate habit \"{k}\"")));
                    }
                    out.push((k, v));
                }
                Ok(Entries(out))
            }
        }
        d.deserialize_map(V)
    }
}

fn malformed(why: impl Into<String>) -> StoreError {
    StoreError::Malformed(why.into())
}

impl JsonFileStore<'_> {
    fn parse(&self, text: &str) -> Result<HabitBook, StoreError> {
        let value: serde_json::Value = serde_json::from_str(text).map_err(|e| malformed(format!("invalid JSON: {e}")))?;
        let obj = value.as_object().ok_or_else(|| malformed("the top level must be a JSON object"))?;
        match obj.get("version") {
            None => return Err(malformed("missing \"version\"")),
            Some(v) => match v.as_u64() {
                None => return Err(malformed("\"version\" must be a non-negative integer")),
                Some(1) => {}
                Some(n) => return Err(StoreError::UnknownVersion(n)),
            },
        }
        let file: FileV1 = serde_json::from_str(text).map_err(|e| malformed(e.to_string()))?;
        let mut entries = Vec::new();
        for (key, entry) in file.habits.0 {
            let name = HabitName::parse(&key).map_err(|e| malformed(format!("bad habit name \"{key}\": {e:?}")))?;
            let mut days = Vec::new();
            for d in &entry.checkins {
                days.push(
                    self.dates
                        .parse(d)
                        .ok_or_else(|| malformed(format!("habit \"{key}\" has a bad date \"{d}\"")))?,
                );
            }
            entries.push((name, days));
        }
        HabitBook::from_entries(entries).map_err(|dup| {
            malformed(format!("habits \"{}\" and \"{}\" differ only in case", dup.first, dup.second))
        })
    }

    fn render(&self, book: &HabitBook) -> String {
        let mut habits = serde_json::Map::new();
        for h in book.habits() {
            let checkins: Vec<serde_json::Value> =
                h.days.iter().map(|d| serde_json::Value::String(self.dates.format(*d))).collect();
            let mut entry = serde_json::Map::new();
            entry.insert("checkins".into(), serde_json::Value::Array(checkins));
            habits.insert(h.name.display().to_string(), serde_json::Value::Object(entry));
        }
        let mut root = serde_json::Map::new();
        root.insert("version".into(), serde_json::Value::from(1));
        root.insert("habits".into(), serde_json::Value::Object(habits));
        let mut text = serde_json::to_string_pretty(&serde_json::Value::Object(root)).expect("a map serializes");
        text.push('\n');
        text
    }
}

impl HabitStore for JsonFileStore<'_> {
    fn load(&self) -> Result<HabitBook, StoreError> {
        match std::fs::symlink_metadata(&self.path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return if self.is_default { Ok(HabitBook::default()) } else { Err(StoreError::NotFound) };
            }
            Err(e) => return Err(StoreError::Io(e.to_string())),
            Ok(_) => {}
        }
        let text = std::fs::read_to_string(&self.path).map_err(|e| StoreError::Io(e.to_string()))?;
        self.parse(&text)
    }

    fn save(&self, book: &HabitBook) -> Result<(), StoreError> {
        let dir = match self.path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        };
        if !dir.is_dir() {
            if self.is_default {
                std::fs::create_dir_all(&dir).map_err(|e| StoreError::Io(e.to_string()))?;
            } else {
                return Err(StoreError::Io(format!("the directory {} does not exist", dir.display())));
            }
        }
        let name = self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
        let write = || -> std::io::Result<()> {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(self.render(book).as_bytes())?;
            f.sync_all()?;
            std::fs::rename(&tmp, &self.path)
        };
        write().map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            StoreError::Io(e.to_string())
        })
    }
}
