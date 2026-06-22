use crate::config::Config;
use crate::fsutil::{files_identical, find_free_path, move_file, sanitise_dir_name};
use crate::report::{ActionOutcome, ExecutionReport, Plan, PlannedAction, PlannedMove, SkipReason};
use crate::store::PersonTarget;
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
/// `config.case_insensitive` is true) and map to a `PersonTarget`.
pub fn build_plan(
    source_dir: &Path,
    config: &Config,
    alias_map: &HashMap<String, PersonTarget>,
    dest_root: &Path,
) -> Result<Plan, EngineError> {
    let extensions: HashSet<String> =
        config.extensions.iter().map(|e| e.to_lowercase()).collect();

    let mut plan = Plan::default();
    let mut reserved: HashSet<PathBuf> = HashSet::new();

    // Only skip files already under dest_root when it is nested inside source_dir.
    // When sorting in place (dest_root == source_dir) nothing is blanket-skipped;
    // already-sorted files are caught by duplicate detection below.
    let dest_is_nested = dest_root != source_dir && dest_root.starts_with(source_dir);

    for entry in WalkDir::new(source_dir)
        .min_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();

        if dest_is_nested && path.starts_with(dest_root) {
            continue;
        }

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

        let haystack = if config.case_insensitive {
            filename.to_lowercase()
        } else {
            filename.to_string()
        };

        let mut matched_target: Option<&PersonTarget> = None;
        let mut ambiguous = false;
        for (alias, target) in alias_map {
            if haystack.contains(alias.as_str()) {
                match matched_target {
                    None => matched_target = Some(target),
                    Some(existing) if existing.name == target.name => {}
                    Some(_) => {
                        ambiguous = true;
                        break;
                    }
                }
            }
        }

        let target = match (matched_target, ambiguous) {
            (_, true) | (None, _) => {
                plan.actions.push(PlannedAction::Skip {
                    src: path.to_path_buf(),
                    reason: SkipReason::UnknownUsername(filename.to_string()),
                });
                continue;
            }
            (Some(t), false) => t,
        };

        let category = target.category.as_deref().unwrap_or("Uncategorised");
        let category_dir = sanitise_dir_name(category);
        let person_dir = sanitise_dir_name(&target.name);
        let desired_dst = dest_root
            .join(&category_dir)
            .join(&person_dir)
            .join(filename);

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
                        name: target.name.clone(),
                    }));
                }
                Err(e) => {
                    eprintln!("Warning: cannot compare '{}': {e}", path.display());
                    continue;
                }
            }
        } else {
            let dst = find_free_path(&desired_dst, &reserved);
            reserved.insert(dst.clone());
            plan.actions.push(PlannedAction::Move(PlannedMove {
                src: path.to_path_buf(),
                dst,
                name: target.name.clone(),
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

    fn alias_map(entries: &[(&str, &str, Option<&str>)]) -> HashMap<String, PersonTarget> {
        entries
            .iter()
            .map(|(alias, name, cat)| {
                (
                    alias.to_string(),
                    PersonTarget {
                        name: name.to_string(),
                        category: cat.map(str::to_string),
                    },
                )
            })
            .collect()
    }

    fn write_file(path: &Path, content: &[u8]) {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    fn test_config() -> Config {
        Config {
            destination_root: None,
            case_insensitive: true,
            extensions: vec!["jpg".into(), "png".into()],
            database: None,
            sort_in_place: false,
        }
    }

    #[test]
    fn known_file_planned_for_move() {
        let inbox = tempdir().unwrap();
        let dest = tempdir().unwrap();
        write_file(&inbox.path().join("joebloggs-20251203.jpg"), b"img");

        let map = alias_map(&[("joebloggs", "Joe Bloggs", Some("Friends"))]);
        let config = test_config();
        let plan = build_plan(inbox.path(), &config, &map, dest.path()).unwrap();

        assert_eq!(plan.actions.len(), 1);
        assert!(matches!(plan.actions[0], PlannedAction::Move(_)));
    }

    #[test]
    fn file_lands_in_category_folder() {
        let inbox = tempdir().unwrap();
        let dest = tempdir().unwrap();
        write_file(&inbox.path().join("joebloggs-20251203.jpg"), b"img");

        let map = alias_map(&[("joebloggs", "Joe Bloggs", Some("Friends"))]);
        let config = test_config();
        let plan = build_plan(inbox.path(), &config, &map, dest.path()).unwrap();

        if let PlannedAction::Move(m) = &plan.actions[0] {
            let expected = dest.path().join("Friends").join("Joe Bloggs").join("joebloggs-20251203.jpg");
            assert_eq!(m.dst, expected);
        } else {
            panic!("expected Move action");
        }
    }

    #[test]
    fn no_category_uses_uncategorised_folder() {
        let inbox = tempdir().unwrap();
        let dest = tempdir().unwrap();
        write_file(&inbox.path().join("janedoe-20251203.jpg"), b"img");

        let map = alias_map(&[("janedoe", "Jane Doe", None)]);
        let config = test_config();
        let plan = build_plan(inbox.path(), &config, &map, dest.path()).unwrap();

        if let PlannedAction::Move(m) = &plan.actions[0] {
            let expected = dest.path().join("Uncategorised").join("Jane Doe").join("janedoe-20251203.jpg");
            assert_eq!(m.dst, expected);
        } else {
            panic!("expected Move action");
        }
    }

    #[test]
    fn unknown_username_leaves_file_in_place() {
        let inbox = tempdir().unwrap();
        let dest = tempdir().unwrap();
        write_file(&inbox.path().join("unknown-20251203.jpg"), b"img");

        let map = alias_map(&[("joebloggs", "Joe Bloggs", None)]);
        let config = test_config();
        let plan = build_plan(inbox.path(), &config, &map, dest.path()).unwrap();

        assert!(matches!(
            plan.actions[0],
            PlannedAction::Skip { reason: SkipReason::UnknownUsername(_), .. }
        ));
    }

    #[test]
    fn ambiguous_match_leaves_file_in_place() {
        let inbox = tempdir().unwrap();
        let dest = tempdir().unwrap();
        write_file(&inbox.path().join("joe-bloggs-20251203.jpg"), b"img");

        let map = alias_map(&[
            ("joe", "Joe Bloggs", None),
            ("joe-bloggs", "Jane Doe", None),
        ]);
        let config = test_config();
        let plan = build_plan(inbox.path(), &config, &map, dest.path()).unwrap();

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
            &dest.path().join("Friends").join("Joe Bloggs").join("joebloggs-20251203.jpg"),
            content,
        );

        let map = alias_map(&[("joebloggs", "Joe Bloggs", Some("Friends"))]);
        let config = test_config();
        let plan = build_plan(inbox.path(), &config, &map, dest.path()).unwrap();

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
            &dest.path().join("Friends").join("Joe Bloggs").join("joebloggs-20251203.jpg"),
            b"existing image",
        );

        let map = alias_map(&[("joebloggs", "Joe Bloggs", Some("Friends"))]);
        let config = test_config();
        let plan = build_plan(inbox.path(), &config, &map, dest.path()).unwrap();

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

        let map = alias_map(&[("joebloggs", "Joe Bloggs", Some("Friends"))]);
        let config = test_config();
        let plan = build_plan(inbox.path(), &config, &map, dest.path()).unwrap();
        let report = execute_plan(&plan);

        assert_eq!(report.moved(), 1);
        assert!(!inbox.path().join("joebloggs-20251203.jpg").exists());
        assert!(dest
            .path()
            .join("Friends")
            .join("Joe Bloggs")
            .join("joebloggs-20251203.jpg")
            .exists());
    }

    #[test]
    fn non_image_extensions_ignored() {
        let inbox = tempdir().unwrap();
        let dest = tempdir().unwrap();
        write_file(&inbox.path().join("joebloggs-20251203.txt"), b"text");

        let map = alias_map(&[("joebloggs", "Joe Bloggs", None)]);
        let config = test_config();
        let plan = build_plan(inbox.path(), &config, &map, dest.path()).unwrap();

        assert!(plan.actions.is_empty());
    }

    #[test]
    fn sort_in_place_plans_move_into_source() {
        let dir = tempdir().unwrap();
        write_file(&dir.path().join("joebloggs-20251203.jpg"), b"img");

        let map = alias_map(&[("joebloggs", "Joe Bloggs", Some("Friends"))]);
        let config = test_config();
        let plan = build_plan(dir.path(), &config, &map, dir.path()).unwrap();

        assert_eq!(plan.actions.len(), 1);
        if let PlannedAction::Move(m) = &plan.actions[0] {
            let expected = dir.path().join("Friends").join("Joe Bloggs").join("joebloggs-20251203.jpg");
            assert_eq!(m.dst, expected);
        } else {
            panic!("expected Move action");
        }
    }

    #[test]
    fn sort_in_place_rerun_skips_already_sorted() {
        let dir = tempdir().unwrap();
        let content = b"image bytes";
        write_file(
            &dir.path().join("Friends").join("Joe Bloggs").join("joebloggs-20251203.jpg"),
            content,
        );

        let map = alias_map(&[("joebloggs", "Joe Bloggs", Some("Friends"))]);
        let config = test_config();
        let plan = build_plan(dir.path(), &config, &map, dir.path()).unwrap();

        assert!(matches!(
            plan.actions[0],
            PlannedAction::Skip { reason: SkipReason::Duplicate, .. }
        ));
    }

    #[test]
    fn nested_separate_dest_still_skipped() {
        let source = tempdir().unwrap();
        let dest = source.path().join("sorted");
        std::fs::create_dir_all(&dest).unwrap();
        write_file(&dest.join("joebloggs-already.jpg"), b"pre-sorted");

        let map = alias_map(&[("joebloggs", "Joe Bloggs", Some("Friends"))]);
        let config = test_config();
        let plan = build_plan(source.path(), &config, &map, &dest).unwrap();

        assert!(plan.actions.is_empty(), "pre-sorted file in nested dest should be skipped");
    }
}
