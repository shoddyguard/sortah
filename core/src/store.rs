use rusqlite::{params, Connection};
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
    pub name: String,
    pub category: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PersonTarget {
    pub name: String,
    pub category: Option<String>,
}

#[derive(Debug)]
pub struct Alias {
    pub alias: String,
    pub name: String,
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
    pub name_a: String,
    pub name_b: String,
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
                 name TEXT NOT NULL UNIQUE,
                 category  TEXT
             );

             CREATE TABLE IF NOT EXISTS aliases (
                 alias      TEXT PRIMARY KEY,
                 person_id  INTEGER NOT NULL REFERENCES people(id) ON DELETE CASCADE
             );

             CREATE INDEX IF NOT EXISTS idx_aliases_person ON aliases(person_id);",
        )?;

        // Additive migration: add the category column to databases created before this version.
        let has_category: bool = self
            .conn
            .prepare("PRAGMA table_info(people)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .any(|r| r.map(|n| n == "category").unwrap_or(false));
        if !has_category {
            self.conn
                .execute_batch("ALTER TABLE people ADD COLUMN category TEXT;")?;
        }

        Ok(())
    }

    // ---- People ----

    pub fn add_person(&self, name: &str, category: Option<&str>) -> Result<i64, StoreError> {
        self.conn
            .execute(
                "INSERT INTO people (name, category) VALUES (?1, ?2)",
                params![name, category],
            )
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(ref err, _)
                    if err.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    e.into()
                }
                other => StoreError::Sqlite(other),
            })?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn remove_person(&self, name: &str) -> Result<(), StoreError> {
        let rows =
            self.conn
                .execute("DELETE FROM people WHERE name = ?1", params![name])?;
        if rows == 0 {
            return Err(StoreError::PersonNotFound(name.to_string()));
        }
        Ok(())
    }

    /// Set (or clear) the category for a person. Pass `None` to remove the category.
    pub fn set_category(&self, name: &str, category: Option<&str>) -> Result<(), StoreError> {
        let rows = self.conn.execute(
            "UPDATE people SET category = ?2 WHERE name = ?1",
            params![name, category],
        )?;
        if rows == 0 {
            return Err(StoreError::PersonNotFound(name.to_string()));
        }
        Ok(())
    }

    pub fn list_people(&self) -> Result<Vec<Person>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, category FROM people ORDER BY name")?;
        let people = stmt
            .query_map([], |row| {
                Ok(Person {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    category: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(people)
    }

    pub fn get_person(&self, name: &str) -> Result<Person, StoreError> {
        self.conn
            .query_row(
                "SELECT id, name, category FROM people WHERE name = ?1",
                params![name],
                |row| {
                    Ok(Person {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        category: row.get(2)?,
                    })
                },
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    StoreError::PersonNotFound(name.to_string())
                }
                other => StoreError::Sqlite(other),
            })
    }

    // ---- Aliases ----

    pub fn add_alias(&self, name: &str, alias: &str) -> Result<(), StoreError> {
        let person = self.get_person(name)?;
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

    /// List all aliases, optionally filtered to a specific person (by name).
    pub fn list_aliases(&self, name: Option<&str>) -> Result<Vec<Alias>, StoreError> {
        if let Some(name) = name {
            let person = self.get_person(name)?;
            let mut stmt = self.conn.prepare(
                "SELECT a.alias, p.name
                 FROM aliases a JOIN people p ON p.id = a.person_id
                 WHERE a.person_id = ?1
                 ORDER BY a.alias",
            )?;
            let aliases = stmt
                .query_map(params![person.id], |row| {
                    Ok(Alias {
                        alias: row.get(0)?,
                        name: row.get(1)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(aliases)
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT a.alias, p.name
                 FROM aliases a JOIN people p ON p.id = a.person_id
                 ORDER BY p.name, a.alias",
            )?;
            let aliases = stmt
                .query_map([], |row| {
                    Ok(Alias {
                        alias: row.get(0)?,
                        name: row.get(1)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(aliases)
        }
    }

    // ---- Alias map for sorting ----

    /// Build an in-memory map of (normalised alias) -> PersonTarget for use during sort.
    /// When `case_insensitive` is true, keys are lowercased; stored aliases are not changed.
    /// Each person's own name is included as an implicit alias.
    /// Explicit aliases take precedence over the implicit name entry on collision.
    pub fn load_alias_map(
        &self,
        case_insensitive: bool,
    ) -> Result<HashMap<String, PersonTarget>, StoreError> {
        let mut map: HashMap<String, PersonTarget> = HashMap::new();

        // Seed with each person's own name as an implicit alias.
        let mut people_stmt = self
            .conn
            .prepare("SELECT name, category FROM people")?;
        let people_rows = people_stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        for row in people_rows {
            let (name, category) = row?;
            let key = if case_insensitive {
                name.to_lowercase()
            } else {
                name.clone()
            };
            map.insert(key, PersonTarget { name, category });
        }

        // Overlay explicit aliases — these take precedence over the implicit name entry.
        let mut stmt = self.conn.prepare(
            "SELECT a.alias, p.name, p.category
             FROM aliases a JOIN people p ON p.id = a.person_id",
        )?;
        let rows = stmt.query_map([], |row| {
            let alias: String = row.get(0)?;
            let name: String = row.get(1)?;
            let category: Option<String> = row.get(2)?;
            Ok((alias, name, category))
        })?;
        for row in rows {
            let (alias, name, category) = row?;
            let key = if case_insensitive {
                alias.to_lowercase()
            } else {
                alias
            };
            map.insert(key, PersonTarget { name, category });
        }
        Ok(map)
    }

    // ---- CSV import / export ----

    /// Bulk-import from a CSV file. Two formats are accepted:
    ///
    /// New format (header starts with "category"):
    ///   category,name,alias1,alias2,...
    ///
    /// Legacy format (header starts with "name"):
    ///   name,alias1,alias2,...
    ///
    /// People are created automatically if they do not already exist.
    pub fn import_csv(&self, path: &Path) -> Result<ImportResult, StoreError> {
        let mut rdr = csv::ReaderBuilder::new()
            .has_headers(true)
            .flexible(true)
            .from_path(path)?;

        let headers = rdr.headers()?.clone();
        let first_header = headers.get(0).unwrap_or("").to_lowercase();
        let new_format = first_header == "category";

        let mut result = ImportResult::default();

        for record_result in rdr.records() {
            match record_result {
                Ok(record) => {
                    let (category, name, alias_start) = if new_format {
                        let cat = record.get(0).filter(|s| !s.is_empty()).map(str::to_string);
                        let n = match record.get(1) {
                            Some(n) if !n.is_empty() => n.to_string(),
                            _ => {
                                result.errors.push("Row missing name".to_string());
                                continue;
                            }
                        };
                        (cat, n, 2)
                    } else {
                        let n = match record.get(0) {
                            Some(n) if !n.is_empty() => n.to_string(),
                            _ => {
                                result.errors.push("Row missing name".to_string());
                                continue;
                            }
                        };
                        (None, n, 1)
                    };

                    if self.get_person(&name).is_err() {
                        if let Err(e) = self.add_person(&name, category.as_deref()) {
                            result.errors.push(format!(
                                "Could not create person '{}': {}",
                                name, e
                            ));
                            continue;
                        }
                    } else if let Some(ref cat) = category {
                        // Update category if the person already exists and a category is given.
                        let _ = self.set_category(&name, Some(cat.as_str()));
                    }

                    for alias in record.iter().skip(alias_start).filter(|a| !a.is_empty()) {
                        match self.add_alias(&name, alias) {
                            Ok(()) => result.imported += 1,
                            Err(StoreError::DuplicateAlias(_)) => result.skipped_duplicate += 1,
                            Err(e) => result.errors.push(format!(
                                "Could not add alias '{}' for '{}': {}",
                                alias, name, e
                            )),
                        }
                    }
                }
                Err(e) => result.errors.push(format!("CSV parse error: {e}")),
            }
        }
        Ok(result)
    }

    /// Export all aliases to a CSV file.
    /// Format: category,name,alias1,alias2,...
    /// People with no aliases are omitted (consistent with the previous behaviour).
    pub fn export_csv(&self, path: &Path) -> Result<(), StoreError> {
        // Fetch aliases joined with the person's category so we have it per row.
        let mut stmt = self.conn.prepare(
            "SELECT a.alias, p.name, p.category
             FROM aliases a JOIN people p ON p.id = a.person_id
             ORDER BY p.name, a.alias",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut wtr = csv::WriterBuilder::new().flexible(true).from_path(path)?;
        wtr.write_record(["category", "name", "aliases"])?;

        // Stream-group by person name (rows are already sorted by name).
        let mut current_name = String::new();
        let mut record: Vec<String> = Vec::new();

        for (alias, name, category) in rows {
            if name != current_name {
                if !record.is_empty() {
                    wtr.write_record(&record)?;
                }
                current_name = name.clone();
                let cat_str = category.unwrap_or_default();
                record = vec![cat_str, name, alias];
            } else {
                record.push(alias);
            }
        }
        if !record.is_empty() {
            wtr.write_record(&record)?;
        }
        wtr.flush()?;
        Ok(())
    }

    // ---- Validation helpers ----

    /// Find aliases that differ only by case.
    pub fn find_case_collisions(&self) -> Result<Vec<CaseCollision>, StoreError> {
        let aliases = self.list_aliases(None)?;
        let mut lower_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for a in aliases {
            lower_map
                .entry(a.alias.to_lowercase())
                .or_default()
                .push((a.alias, a.name));
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
                        name_a: group[i].1.clone(),
                        name_b: group[j].1.clone(),
                    });
                }
            }
        }
        Ok(collisions)
    }
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
        s.add_person("Joe Bloggs", None).unwrap();
        let people = s.list_people().unwrap();
        assert_eq!(people.len(), 1);
        assert_eq!(people[0].name, "Joe Bloggs");
        assert_eq!(people[0].category, None);
    }

    #[test]
    fn add_person_with_category() {
        let s = store();
        s.add_person("Joe Bloggs", Some("Friends")).unwrap();
        let p = s.get_person("Joe Bloggs").unwrap();
        assert_eq!(p.category.as_deref(), Some("Friends"));
    }

    #[test]
    fn set_category_updates_person() {
        let s = store();
        s.add_person("Joe Bloggs", None).unwrap();
        s.set_category("Joe Bloggs", Some("Family")).unwrap();
        let p = s.get_person("Joe Bloggs").unwrap();
        assert_eq!(p.category.as_deref(), Some("Family"));
    }

    #[test]
    fn set_category_clears_category() {
        let s = store();
        s.add_person("Joe Bloggs", Some("Friends")).unwrap();
        s.set_category("Joe Bloggs", None).unwrap();
        let p = s.get_person("Joe Bloggs").unwrap();
        assert_eq!(p.category, None);
    }

    #[test]
    fn set_category_unknown_person_errors() {
        let s = store();
        let err = s.set_category("Nobody", Some("Friends")).unwrap_err();
        assert!(matches!(err, StoreError::PersonNotFound(_)));
    }

    #[test]
    fn add_alias_verbatim() {
        let s = store();
        s.add_person("Joe Bloggs", None).unwrap();
        s.add_alias("Joe Bloggs", "joeBloggs").unwrap();
        s.add_alias("Joe Bloggs", "joe_bloggs").unwrap();
        let aliases = s.list_aliases(Some("Joe Bloggs")).unwrap();
        assert_eq!(aliases.len(), 2);
        assert!(aliases.iter().any(|a| a.alias == "joeBloggs"));
        assert!(aliases.iter().any(|a| a.alias == "joe_bloggs"));
    }

    #[test]
    fn duplicate_alias_is_rejected() {
        let s = store();
        s.add_person("Joe Bloggs", None).unwrap();
        s.add_alias("Joe Bloggs", "joeBloggs").unwrap();
        let err = s.add_alias("Joe Bloggs", "joeBloggs").unwrap_err();
        assert!(matches!(err, StoreError::DuplicateAlias(_)));
    }

    #[test]
    fn alias_map_carries_category() {
        let s = store();
        s.add_person("Joe Bloggs", Some("Friends")).unwrap();
        s.add_alias("Joe Bloggs", "joeBloggs").unwrap();
        let map = s.load_alias_map(true).unwrap();
        let target = map.get("joebloggs").unwrap();
        assert_eq!(target.name, "Joe Bloggs");
        assert_eq!(target.category.as_deref(), Some("Friends"));
    }

    #[test]
    fn alias_map_no_category() {
        let s = store();
        s.add_person("Jane Doe", None).unwrap();
        let map = s.load_alias_map(true).unwrap();
        let target = map.get("jane doe").unwrap();
        assert_eq!(target.name, "Jane Doe");
        assert_eq!(target.category, None);
    }

    #[test]
    fn alias_map_case_insensitive() {
        let s = store();
        s.add_person("Joe Bloggs", None).unwrap();
        s.add_alias("Joe Bloggs", "joeBloggs").unwrap();
        let map = s.load_alias_map(true).unwrap();
        assert!(map.contains_key("joebloggs"));
        assert!(!map.contains_key("joeBloggs"));
    }

    #[test]
    fn person_name_is_implicit_alias() {
        let s = store();
        s.add_person("joebloggs", None).unwrap();
        let map = s.load_alias_map(false).unwrap();
        assert_eq!(map.get("joebloggs").map(|t| t.name.as_str()), Some("joebloggs"));
    }

    #[test]
    fn explicit_alias_takes_precedence_over_implicit_name() {
        let s = store();
        s.add_person("Joe Bloggs", None).unwrap();
        s.add_alias("Joe Bloggs", "Joe Bloggs").unwrap();
        let map = s.load_alias_map(false).unwrap();
        assert_eq!(map.get("Joe Bloggs").map(|t| t.name.as_str()), Some("Joe Bloggs"));
    }

    #[test]
    fn alias_map_case_sensitive() {
        let s = store();
        s.add_person("Joe Bloggs", None).unwrap();
        s.add_alias("Joe Bloggs", "joeBloggs").unwrap();
        let map = s.load_alias_map(false).unwrap();
        assert!(map.contains_key("joeBloggs"));
        assert!(!map.contains_key("joebloggs"));
    }

    #[test]
    fn csv_import_new_format() {
        let s = store();
        let mut csv = NamedTempFile::new().unwrap();
        writeln!(csv, "category,name,aliases").unwrap();
        writeln!(csv, "Friends,Joe Bloggs,joeBloggs,joe_bloggs").unwrap();
        writeln!(csv, ",Jane Doe,janedoe").unwrap();

        let result = s.import_csv(csv.path()).unwrap();
        assert_eq!(result.imported, 3);
        assert!(result.errors.is_empty());

        let joe = s.get_person("Joe Bloggs").unwrap();
        assert_eq!(joe.category.as_deref(), Some("Friends"));
        let jane = s.get_person("Jane Doe").unwrap();
        assert_eq!(jane.category, None);
    }

    #[test]
    fn csv_import_legacy_format() {
        let s = store();
        let mut csv = NamedTempFile::new().unwrap();
        writeln!(csv, "name,aliases").unwrap();
        writeln!(csv, "Joe Bloggs,joeBloggs,joe_bloggs").unwrap();
        writeln!(csv, "Jane Doe,janedoe").unwrap();

        let result = s.import_csv(csv.path()).unwrap();
        assert_eq!(result.imported, 3);
        assert!(result.errors.is_empty());
        // Legacy format: no category
        let joe = s.get_person("Joe Bloggs").unwrap();
        assert_eq!(joe.category, None);
    }

    #[test]
    fn csv_export_round_trips() {
        let s = store();
        let mut csv = NamedTempFile::new().unwrap();
        writeln!(csv, "category,name,aliases").unwrap();
        writeln!(csv, "Friends,Joe Bloggs,joeBloggs,joe_bloggs").unwrap();
        writeln!(csv, ",Jane Doe,janedoe").unwrap();

        s.import_csv(csv.path()).unwrap();

        let out = NamedTempFile::new().unwrap();
        s.export_csv(out.path()).unwrap();
        let content = std::fs::read_to_string(out.path()).unwrap();
        assert!(content.contains("Joe Bloggs") && content.contains("joeBloggs"));
        assert!(content.contains("Friends"));
        assert!(content.contains("Jane Doe") && content.contains("janedoe"));
    }

    #[test]
    fn case_collision_detected() {
        let s = store();
        s.add_person("Joe Bloggs", None).unwrap();
        s.add_alias("Joe Bloggs", "joeBloggs").unwrap();
        s.add_alias("Joe Bloggs", "joebloggs").unwrap();
        let collisions = s.find_case_collisions().unwrap();
        assert_eq!(collisions.len(), 1);
    }

    #[test]
    fn additive_migration_adds_category_column() {
        // Simulate an old-style DB that lacks the category column.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
             CREATE TABLE aliases (alias TEXT PRIMARY KEY, person_id INTEGER NOT NULL REFERENCES people(id) ON DELETE CASCADE);
             INSERT INTO people (name) VALUES ('Joe Bloggs');",
        )
        .unwrap();
        // Wrap in a Store via the same connection path isn't possible directly,
        // so instead exercise migrate() by opening a tempfile DB that we pre-seed.
        use tempfile::NamedTempFile;
        let f = NamedTempFile::new().unwrap();
        {
            let conn2 = Connection::open(f.path()).unwrap();
            conn2.execute_batch(
                "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
                 CREATE TABLE aliases (alias TEXT PRIMARY KEY, person_id INTEGER NOT NULL REFERENCES people(id) ON DELETE CASCADE);
                 INSERT INTO people (name) VALUES ('Joe Bloggs');",
            ).unwrap();
        }
        // Opening via Store should run the additive migration without error.
        let s = Store::open(f.path()).unwrap();
        let people = s.list_people().unwrap();
        assert_eq!(people.len(), 1);
        assert_eq!(people[0].name, "Joe Bloggs");
        assert_eq!(people[0].category, None);
    }
}
