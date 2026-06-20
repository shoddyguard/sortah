# Agent context for sortah

## What this project is

A CLI tool that sorts downloaded images into per-person directories by extracting a
username from each filename (via a configurable regex) and resolving it through an
alias mapping. One person may have many usernames; all their images land in the same
directory.

## Workspace layout

```
core/   sortah-core  — all business logic; no assumptions about UI
cli/    sortah       — thin clap shell over core; the only user-facing binary
```

The split is intentional: a future GUI becomes a second crate that depends on `core`
without touching `cli`. Do not move logic into `cli`.

## Key design decisions (do not reverse without discussion)

- **Config format is YAML** (`config.yaml`). Not TOML. Do not suggest or introduce TOML.
- **Aliases are stored verbatim** in SQLite. `joeBloggs` is stored as `joeBloggs`.
  The `case_insensitive` setting only affects the *comparison* at sort time, not storage.
- **Settings live in a YAML file** (human-editable, opened in a text editor). They do
  not live in the SQLite database. The database holds only the people/alias mapping.
- **The sort engine has a plan/execute split** (`engine::build_plan` then
  `engine::execute_plan`). The confirmation prompt sits between them in the CLI.
  Keep this split clean — it is what lets the future GUI show a preview before committing.
- **`sortah sort` operates on the current working directory**, not a configured inbox.
  The user `cd`s to wherever the images are.
- **Unknown usernames are left in place** and reported; they are never moved to an
  "unsorted" folder.

## Module responsibilities

| Module | Does |
|---|---|
| `core::config` | Load/validate/template the YAML settings file; tilde expansion |
| `core::store` | SQLite schema + migrations; all people/alias CRUD; CSV import/export; build alias map |
| `core::parse` | Compile the regex; extract `username` named group from a filename |
| `core::engine` | Walk cwd, build `Plan`, execute `Plan`; clash resolution |
| `core::fsutil` | `move_file` (with cross-device fallback); `files_identical`; `find_free_path`; `sanitise_dir_name` |
| `core::report` | `Plan`, `PlannedAction`, `PlanSummary`, `ExecutionReport` types |
| `cli::cli` | clap struct/enum definitions only — no logic |
| `cli::main` | Wire CLI args to core; print output; handle the confirm prompt |

## SQLite schema

```sql
CREATE TABLE people  (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
CREATE TABLE aliases (alias TEXT PRIMARY KEY,
                      person_id INTEGER NOT NULL REFERENCES people(id) ON DELETE CASCADE);
```

`aliases.alias` is the PRIMARY KEY — exact-duplicate aliases are rejected at insert time.
Case-only collisions (e.g. `joeBloggs` vs `joebloggs`) are allowed in the schema but
flagged by `config validate` as a warning when `case_insensitive` is on.

## Clash resolution

When a destination file already exists:
- byte-identical content → skip (recorded as `SkipReason::Duplicate`)
- different content → rename to `file (2).jpg`, `(3)`, etc.

`find_free_path` takes a `&HashSet<PathBuf>` of already-reserved plan destinations so
two files in the same plan cannot be given the same renamed path.

## Building and testing

```sh
cargo build           # dev build
cargo build --release # single static binary at target/release/sortah
cargo test            # 28 unit + integration tests across core modules
```

No external runtime or system library is required: SQLite is compiled in via
`rusqlite`'s `bundled` feature.

## Adding a new feature — checklist

1. Logic goes in `core`; CLI wiring goes in `cli/src/main.rs`.
2. New error variants: use `thiserror` in `core`, `anyhow` context in `cli`.
3. New store operations: add to `store.rs` and expose via `lib.rs` if the CLI needs them.
4. Update `config.yaml` template in `Config::write_template` if new settings are added.
5. Add tests in the relevant `#[cfg(test)]` module; use `tempfile` for filesystem tests
   and `Store::open_in_memory()` for database tests.
