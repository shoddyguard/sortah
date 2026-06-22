use sortah_core::{Config, Store};
use sortah_core::report::Plan;
use std::path::PathBuf;

pub struct AppState {
    pub config_path: Option<PathBuf>,
    pub config: Config,
    pub store: Store,
    pub current_plan: Option<Plan>,
}
