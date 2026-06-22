slint::include_modules!();

mod app;
mod dialogs;
mod models;
mod state;
mod worker;

use anyhow::{Context, Result};
use sortah_core::{Config, Store};
use state::AppState;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

pub fn run(config_path: Option<PathBuf>) -> Result<()> {
    let window = AppWindow::new().context("Failed to create window")?;

    // Resolve the config file path
    let effective_config_path = config_path.or_else(Config::default_path);

    // Load config, falling back to a safe default on any error
    let (config, config_status) = match effective_config_path
        .as_deref()
        .map(Config::load)
        .transpose()
    {
        Ok(Some(cfg)) => (cfg, String::new()),
        _ => {
            let msg = "No config found — using defaults. Go to Config to set up.".to_string();
            let default_cfg = Config {
                destination_root: None,
                sort_in_place: true,
                case_insensitive: true,
                extensions: vec![
                    "jpg".into(), "jpeg".into(), "png".into(),
                    "gif".into(), "webp".into(), "mp4".into(),
                ],
                database: None,
            };
            (default_cfg, msg)
        }
    };

    // Open (or create) the database
    let db_path = config
        .resolved_db_path()
        .or_else(Config::default_db_path)
        .context("Cannot determine database path")?;
    let store = Store::open(&db_path).context("Failed to open database")?;

    let state = Rc::new(RefCell::new(AppState {
        config_path: effective_config_path,
        config,
        store,
        current_plan: None,
    }));

    // Initialise window properties
    app::init_window(&window, &state, &config_status, &db_path);

    // Spawn the worker thread and start the result-drain timer
    let (job_tx, result_rx) = worker::spawn();
    let _timer = app::setup_timer(&window, state.clone(), result_rx);

    // Register all callbacks
    app::register_callbacks(&window, state, job_tx);

    window.run().context("Window error")?;
    Ok(())
}
