use sortah_core::engine::{build_plan, execute_plan};
use sortah_core::report::{ExecutionReport, Plan};
use sortah_core::store::PersonTarget;
use sortah_core::Config;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;

pub enum Job {
    BuildPlan {
        source_dir: PathBuf,
        config: Config,
        alias_map: HashMap<String, PersonTarget>,
        dest_root: PathBuf,
    },
    ExecutePlan {
        plan: Plan,
    },
}

pub enum JobResult {
    PlanReady(Plan),
    PlanFailed(String),
    ExecutionDone(ExecutionReport),
}

pub fn spawn() -> (mpsc::Sender<Job>, mpsc::Receiver<JobResult>) {
    let (job_tx, job_rx) = mpsc::channel::<Job>();
    let (result_tx, result_rx) = mpsc::channel::<JobResult>();

    std::thread::spawn(move || {
        for job in job_rx.iter() {
            match job {
                Job::BuildPlan { source_dir, config, alias_map, dest_root } => {
                    match build_plan(&source_dir, &config, &alias_map, &dest_root) {
                        Ok(plan) => { let _ = result_tx.send(JobResult::PlanReady(plan)); }
                        Err(e)   => { let _ = result_tx.send(JobResult::PlanFailed(e.to_string())); }
                    }
                }
                Job::ExecutePlan { plan } => {
                    let report = execute_plan(&plan);
                    let _ = result_tx.send(JobResult::ExecutionDone(report));
                }
            }
        }
    });

    (job_tx, result_rx)
}
