use crate::{AliasRow, PersonRow, PlanRow};
use slint::{ModelRc, VecModel};
use sortah_core::report::{Plan, PlannedAction};
use sortah_core::store::{Alias, Person};
use std::rc::Rc;

pub fn people_to_model(people: &[Person]) -> ModelRc<PersonRow> {
    let rows: Vec<PersonRow> = people
        .iter()
        .map(|p| PersonRow {
            name: p.name.clone().into(),
            category: p.category.clone().unwrap_or_default().into(),
        })
        .collect();
    ModelRc::from(Rc::new(VecModel::from(rows)))
}

pub fn aliases_to_model(aliases: &[Alias]) -> ModelRc<AliasRow> {
    let rows: Vec<AliasRow> = aliases
        .iter()
        .map(|a| AliasRow { alias: a.alias.clone().into() })
        .collect();
    ModelRc::from(Rc::new(VecModel::from(rows)))
}

pub fn plan_to_model(plan: &Plan) -> ModelRc<PlanRow> {
    let rows: Vec<PlanRow> = plan
        .actions
        .iter()
        .filter_map(|action| match action {
            PlannedAction::Move(m) => {
                let filename = m.src.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                let dest = m.dst.parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                Some(PlanRow {
                    filename: filename.into(),
                    person: m.name.clone().into(),
                    dest: dest.into(),
                })
            }
            _ => None,
        })
        .collect();
    ModelRc::from(Rc::new(VecModel::from(rows)))
}

pub fn empty_plan_model() -> ModelRc<PlanRow> {
    ModelRc::from(Rc::new(VecModel::<PlanRow>::default()))
}

pub fn format_summary(plan: &Plan) -> String {
    let s = plan.summary();
    let mut out = String::new();

    out.push_str(&format!("Files to move: {}", s.to_move));
    if s.skip_duplicate > 0 {
        out.push_str(&format!("\nSkip (duplicate): {}", s.skip_duplicate));
    }
    if s.unknown_total() > 0 {
        out.push_str(&format!("\nSkip (unknown): {}", s.unknown_total()));
    }
    if !s.by_person.is_empty() {
        out.push_str("\n\nBy person:");
        let mut by_person: Vec<_> = s.by_person.iter().collect();
        by_person.sort_by_key(|(n, _)| n.as_str());
        for (name, count) in by_person {
            out.push_str(&format!("\n  {}: {}", name, count));
        }
    }
    if !s.unknown_usernames.is_empty() {
        out.push_str("\n\nUnknown:");
        for (u, count) in &s.unknown_usernames {
            out.push_str(&format!("\n  {} ({} file(s))", u, count));
        }
    }
    out
}
