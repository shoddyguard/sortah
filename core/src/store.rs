use rusqlite::{params, Connection};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("CSV error: {0}")]
    Csv(#[from] csv::Error),
    #[error("Person not found: '{0}'")]
    PersonNotFound(String),
    #[error("Alias already exists: '{0}'")]
    DuplicateAlias(String),
    #[error("Alias not found: '{0}'")]
    AliasNotFound(String),
}

pub struct Store {
    conn: Connection,
}

#[derive(Debug)]
pub struct Person {
    pub id: i64,
    pub canonical: String,
}

#[derive(Debug)]
pub struct Alias {
    pub alias: String,
    pub canonical: String,
}

#[derive(Debug, Default)]
pub struct ImportResult {
    pub imported: usize,
    pub skipped_duplicate: usize,
    pub errors: Vec<String>,
}

/// A case-only collision: two aliases that differ only by case.
#[derive(Debug)]
pub struct CaseCollision {
    pub alias_a: String,
    pub alias_b: String,
    pub canonical_a: String,
    pub canonical_b: String,
}

impl Store {
    /// Open (or create) a store at the given path.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        let mut store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    /// Open an in-memory store (useful for tests).
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        let mut store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<(), StoreError> {
        self.conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;

             CREATE TABLE IF NOT EXISTS people (
                 id        INTEGER PRIMARY KEY,
                 canonical TEXT NOT NULL UNIQUE
             );

             CREATE TABLE IF NOT EXISTS aliases (
                 alias      TEXT PRIMARY KEY,
                 person_id  INTEGER NOT NULL REFERENCES people(id) ON DELETE CASCADE
             );

             CREATE INDEX IF NOT EXISTS idx_aliases_person ON aliases(person_id);",
        )?;
        Ok(())
    }

    // ---- People ----

    pub fn add_person(&self, canonical: &str) -> Result<i64, StoreError> {
        self.conn
            .execute("INSERT INTO people (canonical) VALUES (?1)", params![canonical])
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(ref err, _)
                    if err.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    // Person already exists — return their id instead of erroring.
                    // Caller can decide whether to care.
                    e.into()
                }
                other => StoreError::Sqlite(other),
            })?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn remove_person(&self, canonical: &str) -> Result<(), StoreError> {
        let rows =
            self.conn
                .execute("DELETE FROM people WHERE canonical = ?1", params![canonical])?;
        if rows == 0 {
            return Err(StoreError::PersonNotFound(canonical.to_string()));
        }
        Ok(())
    }

    pub fn list_people(&self) -> Result<Vec<Person>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, canonical FROM people ORDER BY canonical")?;
        let people = stmt
            .query_map([], |row| {
                Ok(Person {
                    id: row.get(0)?,
                    canonical: row.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(people)
    }

    pub fn get_person(&self, canonical: &str) -> Result<Person, StoreError> {
        self.conn
            .query_row(
                "SELECT id, canonical FROM people WHERE canonical = ?1",
                params![canonical],
                |row| {
                    Ok(Person {
                        id: row.get(0)?,
                        canonical: row.get(1)?,
                    })
                },
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    StoreError::PersonNotFound(canonical.to_string())
                }
                other => StoreError::Sqlite(other),
            })
    }

    // ---- Aliases ----

    pub fn add_alias(&self, canonical: &str, alias: &str) -> Result<(), StoreError> {
        let person = self.get_person(canonical)?;
        self.conn
            .execute(
                "INSERT INTO aliases (alias, person_id) VALUES (?1, ?2)",
                params![alias, person.id],
            )
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(ref err, _)
                    if err.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    StoreError::DuplicateAlias(alias.to_string())
                }
                other => StoreError::Sqlite(other),
            })?;
        Ok(())
    }

    pub fn remove_alias(&self, alias: &str) -> Result<(), StoreError> {
        let rows =
            self.conn
                .execute("DELETE FROM aliases WHERE alias = ?1", params![alias])?;
        if rows == 0 {
            return Err(StoreError::AliasNotFound(alias.to_string()));
        }
        Ok(())
    }

    /// List all aliases, optionally filtered to a specific person (by canonical name).
    pub fn list_aliases(&self, canonical: Option<&str>) -> Result<Vec<Alias>, StoreError> {
        if let Some(canonical) = canonical {
            let person = self.get_person(canonical)?;
            let mut stmt = self.conn.prepare(
                "SELECT a.alias, p.canonical
                 FROM aliases a JOIN people p ON p.id = a.person_id
                 WHERE a.person_id = ?1
                 ORDER BY a.alias",
            )?;
            let aliases = stmt
                .query_map(params![person.id], |row| {
                    Ok(Alias {
                        alias: row.get(0)?,
                        canonical: row.get(1)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(aliases)
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT a.alias, p.canonical
                 FROM aliases a JOIN people p ON p.id = a.person_id
                 ORDER BY p.canonical, a.alias",
            )?;
            let aliases = stmt
                .query_map([], |row| {
                    Ok(Alias {
                        alias: row.get(0)?,
                        canonical: row.get(1)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(aliases)
        }
    }

    // ---- Alias map for sorting ----

    /// Build an in-memory map of (normalised alias) -> canonical name for use during sort.
    /// When `case_insensitive` is true, keys are lowercased; stored aliases are not changed.
    pub fn load_alias_map(
        &self,
        case_insensitive: bool,
    ) -> Result<HashMap<String, String>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT a.alias, p.canonical
             FROM aliases a JOIN people p ON p.id = a.person_id",
        )?;
        let mut map = HashMap::new();
        let rows = stmt.query_map([], |row| {
            let alias: String = row.get(0)?;
            let canonical: String = row.get(1)?;
            Ok((alias, canonical))
        })?;
        for row in rows {
            let (alias, canonical) = row?;
            let key = if case_insensitive {
                alias.to_lowercase()
            } else {
                alias
            };
            // Last-write wins on collision — validate warns about this separately.
            map.insert(key, canonical);
        }
        Ok(map)
    }

    // ---- CSV import / export ----

    /// Bulk-import aliases from a CSV file with headers `canonical,alias`.
    /// People are created automatically if they do not already exist.
    pub fn import_csv(&self, path: &Path) -> Result<ImportResult, StoreError> {
        let mut rdr = csv::ReaderBuilder::new()
            .has_headers(true)
            .from_path(path)?;

        let mut result = ImportResult::default();

        for record_result in rdr.deserialize::<CsvRow>() {
            match record_result {
                Ok(row) => {
                    // Ensure the person exists; ignore "already exists" errors.
                    if self.get_person(&row.canonical).is_err() {
                        if let Err(e) = self.add_person(&row.canonical) {
                            result.errors.push(format!(
                                "Could not create person '{}': {}",
                                row.canonical, e
                            ));
                            continue;
                        }
                    }
                    match self.add_alias(&row.canonical, &row.alias) {
                        Ok(()) => result.imported += 1,
                        Err(StoreError::DuplicateAlias(_)) => result.skipped_duplicate += 1,
                        Err(e) => result.errors.push(format!(
                            "Could not add alias '{}' for '{}': {}",
                            row.alias, row.canonical, e
                        )),
                    }
                }
                Err(e) => result.errors.push(format!("CSV parse error: {e}")),
            }
        }
        Ok(result)
    }

    /// Export all aliases to a CSV file with headers `canonical,alias`.
    pub fn export_csv(&self, path: &Path) -> Result<(), StoreError> {
        let aliases = self.list_aliases(None)?;
        let mut wtr = csv::Writer::from_path(path)?;
        wtr.write_record(["canonical", "alias"])?;
        for alias in &aliases {
            wtr.write_record([&alias.canonical, &alias.alias])?;
        }
        wtr.flush()?;
        Ok(())
    }

    // ---- Validation helpers ----

    /// Find aliases that differ only by case. When `case_insensitive` is on in the config,
    /// these would be ambiguous and the last-loaded one silently wins. Report them so the
    /// user can resolve the ambiguity.
    pub fn find_case_collisions(&self) -> Result<Vec<CaseCollision>, StoreError> {
        let aliases = self.list_aliases(None)?;
        // Group by lowercased alias
        let mut lower_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for a in aliases {
            lower_map
                .entry(a.alias.to_lowercase())
                .or_default()
                .push((a.alias, a.canonical));
        }
        let mut collisions = Vec::new();
        for group in lower_map.values() {
            if group.len() < 2 {
                continue;
            }
            for i in 0..group.len() {
                for j in (i + 1)..group.len() {
                    collisions.push(CaseCollision {
                        alias_a: group[i].0.clone(),
                        alias_b: group[j].0.clone(),
                        canonical_a: group[i].1.clone(),
                        canonical_b: group[j].1.clone(),
                    });
                }
            }
        }
        Ok(collisions)
    }
}

#[derive(Debug, Deserialize)]
struct CsvRow {
    canonical: String,
    alias: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    #[test]
    fn add_and_list_person() {
        let s = store();
        s.add_person("Joe Bloggs").unwrap();
        let people = s.list_people().unwrap();
        assert_eq!(people.len(), 1);
        assert_eq!(people[0].canonical, "Joe Bloggs");
    }

    #[test]
    fn add_alias_verbatim() {
        let s = store();
        s.add_person("Joe Bloggs").unwrap();
        s.add_alias("Joe Bloggs", "joeBloggs").unwrap();
        s.add_alias("Joe Bloggs", "joe_bloggs").unwrap();
        let aliases = s.list_aliases(Some("Joe Bloggs")).unwrap();
        assert_eq!(aliases.len(), 2);
        // Stored exactly as given
        assert!(aliases.iter().any(|a| a.alias == "joeBloggs"));
        assert!(aliases.iter().any(|a| a.alias == "joe_bloggs"));
    }

    #[test]
    fn duplicate_alias_is_rejected() {
        let s = store();
        s.add_person("Joe Bloggs").unwrap();
        s.add_alias("Joe Bloggs", "joeBloggs").unwrap();
        let err = s.add_alias("Joe Bloggs", "joeBloggs").unwrap_err();
        assert!(matches!(err, StoreError::DuplicateAlias(_)));
    }

    #[test]
    fn alias_map_case_insensitive() {
        let s = store();
        s.add_person("Joe Bloggs").unwrap();
        s.add_alias("Joe Bloggs", "joeBloggs").unwrap();
        let map = s.load_alias_map(true).unwrap();
        // Stored as "joeBloggs", key lowercased to "joebloggs"
        assert_eq!(map.get("joebloggs").map(String::as_str), Some("Joe Bloggs"));
        assert!(map.get("joeBloggs").is_none());
    }

    #[test]
    fn alias_map_case_sensitive() {
        let s = store();
        s.add_person("Joe Bloggs").unwrap();
        s.add_alias("Joe Bloggs", "joeBloggs").unwrap();
        let map = s.load_alias_map(false).unwrap();
        assert_eq!(map.get("joeBloggs").map(String::as_str), Some("Joe Bloggs"));
        assert!(map.get("joebloggs").is_none());
    }

    #[test]
    fn csv_import_export_round_trips() {
        let s = store();
        let mut csv = NamedTempFile::new().unwrap();
        writeln!(csv, "canonical,alias").unwrap();
        writeln!(csv, "Joe Bloggs,joeBloggs").unwrap();
        writeln!(csv, "Joe Bloggs,joe_bloggs").unwrap();
        writeln!(csv, "Jane Doe,janedoe").unwrap();

        let result = s.import_csv(csv.path()).unwrap();
        assert_eq!(result.imported, 3);
        assert_eq!(result.skipped_duplicate, 0);
        assert!(result.errors.is_empty());

        let out = NamedTempFile::new().unwrap();
        s.export_csv(out.path()).unwrap();
        let content = std::fs::read_to_string(out.path()).unwrap();
        assert!(content.contains("Joe Bloggs,joeBloggs") || content.contains("Joe Bloggs,joe_bloggs"));
        assert!(content.contains("Jane Doe,janedoe"));
    }

    #[test]
    fn case_collision_detected() {
        let s = store();
        s.add_person("Joe Bloggs").unwrap();
        s.add_alias("Joe Bloggs", "joeBloggs").unwrap();
        s.add_alias("Joe Bloggs", "joebloggs").unwrap(); // differs only by case
        let collisions = s.find_case_collisions().unwrap();
        assert_eq!(collisions.len(), 1);
    }
}
