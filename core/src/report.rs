use std::collections::HashMap;
use std::path::PathBuf;

/// Why a file is being left in place.
#[derive(Debug)]
pub enum SkipReason {
    /// An identical file already exists at the destination.
    Duplicate,
    /// No alias in the mapping matched this filename.
    UnknownUsername(String),
}

/// A single planned file move (includes clash-renamed destinations).
#[derive(Debug)]
pub struct PlannedMove {
    pub src: PathBuf,
    pub dst: PathBuf,
    pub name: String,
}

/// One item in the sort plan.
#[derive(Debug)]
pub enum PlannedAction {
    Move(PlannedMove),
    Skip { src: PathBuf, reason: SkipReason },
}

/// The full plan produced by the engine's plan phase (no disk changes made yet).
#[derive(Debug, Default)]
pub struct Plan {
    pub actions: Vec<PlannedAction>,
}

impl Plan {
    pub fn summary(&self) -> PlanSummary {
        let mut summary = PlanSummary::default();
        let mut unknown_counts: HashMap<String, usize> = HashMap::new();

        for action in &self.actions {
            match action {
                PlannedAction::Move(m) => {
                    summary.to_move += 1;
                    *summary.by_person.entry(m.name.clone()).or_insert(0) += 1;
                }
                PlannedAction::Skip { reason, .. } => match reason {
                    SkipReason::Duplicate => summary.skip_duplicate += 1,
                    SkipReason::UnknownUsername(u) => {
                        *unknown_counts.entry(u.clone()).or_insert(0) += 1;
                    }
                },
            }
        }

        // Sort unknown usernames by descending count, then alphabetically.
        let mut unknown: Vec<(String, usize)> = unknown_counts.into_iter().collect();
        unknown.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        summary.unknown_usernames = unknown;
        summary
    }
}

/// Aggregated statistics over a plan, used to display the pre-confirmation summary.
#[derive(Debug, Default)]
pub struct PlanSummary {
    pub to_move: usize,
    pub skip_duplicate: usize,
    pub unknown_usernames: Vec<(String, usize)>,
    /// Files to move per person name.
    pub by_person: HashMap<String, usize>,
}

impl PlanSummary {
    pub fn unknown_total(&self) -> usize {
        self.unknown_usernames.iter().map(|(_, n)| n).sum()
    }
}

/// Outcome of executing one move action.
#[derive(Debug)]
pub enum ActionOutcome {
    Moved { src: PathBuf, dst: PathBuf },
    Failed { src: PathBuf, error: String },
    Skipped { src: PathBuf },
}

/// The result of the execute phase.
#[derive(Debug, Default)]
pub struct ExecutionReport {
    pub outcomes: Vec<ActionOutcome>,
}

impl ExecutionReport {
    pub fn moved(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| matches!(o, ActionOutcome::Moved { .. }))
            .count()
    }

    pub fn failed(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| matches!(o, ActionOutcome::Failed { .. }))
            .count()
    }

    pub fn failures(&self) -> impl Iterator<Item = (&PathBuf, &str)> {
        self.outcomes.iter().filter_map(|o| {
            if let ActionOutcome::Failed { src, error } = o {
                Some((src, error.as_str()))
            } else {
                None
            }
        })
    }
}
