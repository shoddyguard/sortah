use crate::config::Config;
use crate::fsutil::{files_identical, find_free_path, move_file, sanitise_dir_name};
use crate::report::{ActionOutcome, ExecutionReport, Plan, PlannedAction, PlannedMove, SkipReason};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use thiserror::Error;
use walkdir::WalkDir;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("Directory walk error: {0}")]
    Walk(#[from] walkdir::Error),
}

/// Build a sort plan by scanning `source_dir` recursively.
///
/// No files are moved; the returned `Plan` describes every intended action.
/// `alias_map` must be keyed by normalised alias (already lowercased when
/// `config.case_insensitive` is true) and map to the person name.
pub fn build_plan(
    source_dir: &Path,
    config: &Config,
    alias_map: &HashMap<String, String>,
    dest_override: Option<&Path>,
) -> Result<Plan, EngineError> {
    let dest_root = dest_override.unwrap_or(&config.destination_root);

    let extensions: HashSet<String> =
        config.extensions.iter().map(|e| e.to_lowercase()).collect();

    let mut plan = Plan::default();
    // Track destination paths already reserved by this plan to avoid double-booking.
    let mut reserved: HashSet<PathBuf> = HashSet::new();

    for entry in WalkDir::new(source_dir)
        .min_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();

        // Skip files that already live under the destination root.
        if path.starts_with(dest_root) {
            continue;
        }

        // Filter by extension.
        let ext_ok = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| extensions.contains(&e.to_lowercase()))
            .unwrap_or(false);
        if !ext_ok {
            continue;
        }

        let filename = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };

        // Substring match: find any alias that appears in the filename.
        // alias_map keys are already normalised (lowercased if case_insensitive).
        let haystack = if config.case_insensitive {
            filename.to_lowercase()
        } else {
            filename.to_string()
        };

        let mut matched_person: Option<&str> = None;
        let mut ambiguous = false;
        for (alias, person) in alias_map {
            if haystack.contains(alias.as_str()) {
                match matched_person {
                    None => matched_person = Some(person),
                    Some(existing) if existing == person => {}
                    Some(_) => { ambiguous = true; break; }
                }
            }
        }

        let name = match (matched_person, ambiguous) {
            (_, true) | (None, _) => {
                plan.actions.push(PlannedAction::Skip {
                    src: path.to_path_buf(),
                    reason: SkipReason::UnknownUsername(filename.to_string()),
                });
                continue;
            }
            (Some(n), false) => n.to_string(),
        };

        let safe_dir = sanitise_dir_name(&name);
        let desired_dst = dest_root.join(&safe_dir).join(filename);

        // Resolve clashes with existing files or earlier plan entries.
        if desired_dst.exists() {
            match files_identical(path, &desired_dst) {
                Ok(true) => {
                    plan.actions.push(PlannedAction::Skip {
                        src: path.to_path_buf(),
                        reason: SkipReason::Duplicate,
                    });
                    continue;
                }
                Ok(false) => {
                    let dst = find_free_path(&desired_dst, &reserved);
                    reserved.insert(dst.clone());
                    plan.actions.push(PlannedAction::Move(PlannedMove {
                        src: path.to_path_buf(),
                        dst,
                        name,
                    }));
                }
                Err(e) => {
                    eprintln!("Warning: cannot compare '{}': {e}", path.display());
                    // Leave file in place rather than risk overwriting.
                    continue;
                }
            }
        } else {
            let dst = find_free_path(&desired_dst, &reserved);
            reserved.insert(dst.clone());
            plan.actions.push(PlannedAction::Move(PlannedMove {
                src: path.to_path_buf(),
                dst,
                name,
            }));
        }
    }

    Ok(plan)
}

/// Execute the plan, moving files as described.
/// Individual file errors are recorded in the report rather than aborting the whole run.
pub fn execute_plan(plan: &Plan) -> ExecutionReport {
    let mut report = ExecutionReport::default();
    for action in &plan.actions {
        match action {
            PlannedAction::Move(m) => match move_file(&m.src, &m.dst) {
                Ok(()) => report.outcomes.push(ActionOutcome::Moved {
                    src: m.src.clone(),
                    dst: m.dst.clone(),
                }),
                Err(e) => report.outcomes.push(ActionOutcome::Failed {
                    src: m.src.clone(),
                    error: e.to_string(),
                }),
            },
            PlannedAction::Skip { src, .. } => {
                report.outcomes.push(ActionOutcome::Skipped { src: src.clone() });
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn alias_map(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn write_file(path: &Path, content: &[u8]) {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    fn test_config(dest: &Path) -> Config {
        Config {
            destination_root: dest.to_path_buf(),
            case_insensitive: true,
            extensions: vec!["jpg".into(), "png".into()],
            database: None,
        }
    }

    #[test]
    fn known_file_planned_for_move() {
        let inbox = tempdir().unwrap();
        let dest = tempdir().unwrap();
        write_file(&inbox.path().join("joebloggs-20251203.jpg"), b"img");

        let map = alias_map(&[("joebloggs", "Joe Bloggs")]);
        let config = test_config(dest.path());
        let plan = build_plan(inbox.path(), &config, &map, None).unwrap();

        assert_eq!(plan.actions.len(), 1);
        assert!(matches!(plan.actions[0], PlannedAction::Move(_)));
    }

    #[test]
    fn unknown_username_leaves_file_in_place() {
        let inbox = tempdir().unwrap();
        let dest = tempdir().unwrap();
        write_file(&inbox.path().join("unknown-20251203.jpg"), b"img");

        let map = alias_map(&[("joebloggs", "Joe Bloggs")]);
        let config = test_config(dest.path());
        let plan = build_plan(inbox.path(), &config, &map, None).unwrap();

        assert!(matches!(
            plan.actions[0],
            PlannedAction::Skip { reason: SkipReason::UnknownUsername(_), .. }
        ));
    }

    #[test]
    fn ambiguous_match_leaves_file_in_place() {
        let inbox = tempdir().unwrap();
        let dest = tempdir().unwrap();
        // filename contains both "joe" (Joe Bloggs) and "joe-bloggs" (Jane Doe)
        write_file(&inbox.path().join("joe-bloggs-20251203.jpg"), b"img");

        let map = alias_map(&[("joe", "Joe Bloggs"), ("joe-bloggs", "Jane Doe")]);
        let config = test_config(dest.path());
        let plan = build_plan(inbox.path(), &config, &map, None).unwrap();

        assert!(matches!(
            plan.actions[0],
            PlannedAction::Skip { reason: SkipReason::UnknownUsername(_), .. }
        ));
    }

    #[test]
    fn identical_duplicate_skipped() {
        let inbox = tempdir().unwrap();
        let dest = tempdir().unwrap();
        let content = b"image bytes";
        write_file(&inbox.path().join("joebloggs-20251203.jpg"), content);
        write_file(
            &dest.path().join("Joe Bloggs").join("joebloggs-20251203.jpg"),
            content,
        );

        let map = alias_map(&[("joebloggs", "Joe Bloggs")]);
        let config = test_config(dest.path());
        let plan = build_plan(inbox.path(), &config, &map, None).unwrap();

        assert!(matches!(
            plan.actions[0],
            PlannedAction::Skip { reason: SkipReason::Duplicate, .. }
        ));
    }

    #[test]
    fn differing_clash_renamed() {
        let inbox = tempdir().unwrap();
        let dest = tempdir().unwrap();
        write_file(&inbox.path().join("joebloggs-20251203.jpg"), b"new image");
        write_file(
            &dest.path().join("Joe Bloggs").join("joebloggs-20251203.jpg"),
            b"existing image",
        );

        let map = alias_map(&[("joebloggs", "Joe Bloggs")]);
        let config = test_config(dest.path());
        let plan = build_plan(inbox.path(), &config, &map, None).unwrap();

        if let PlannedAction::Move(m) = &plan.actions[0] {
            let name = m.dst.file_name().unwrap().to_str().unwrap();
            assert!(name.contains("(2)"), "expected renamed destination, got {name}");
        } else {
            panic!("expected Move action");
        }
    }

    #[test]
    fn execute_plan_moves_files() {
        let inbox = tempdir().unwrap();
        let dest = tempdir().unwrap();
        write_file(&inbox.path().join("joebloggs-20251203.jpg"), b"img");

        let map = alias_map(&[("joebloggs", "Joe Bloggs")]);
        let config = test_config(dest.path());
        let plan = build_plan(inbox.path(), &config, &map, None).unwrap();
        let report = execute_plan(&plan);

        assert_eq!(report.moved(), 1);
        assert!(!inbox.path().join("joebloggs-20251203.jpg").exists());
        assert!(dest
            .path()
            .join("Joe Bloggs")
            .join("joebloggs-20251203.jpg")
            .exists());
    }

    #[test]
    fn non_image_extensions_ignored() {
        let inbox = tempdir().unwrap();
        let dest = tempdir().unwrap();
        write_file(&inbox.path().join("joebloggs-20251203.txt"), b"text");

        let map = alias_map(&[("joebloggs", "Joe Bloggs")]);
        let config = test_config(dest.path());
        let plan = build_plan(inbox.path(), &config, &map, None).unwrap();

        assert!(plan.actions.is_empty());
    }
}
