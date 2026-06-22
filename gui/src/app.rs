use crate::state::AppState;
use crate::{dialogs, models, AppWindow};
use crate::worker::{Job, JobResult};
use slint::{ComponentHandle, Model};
use sortah_core::{Config, Store};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;

// ── Startup initialisation ────────────────────────────────────────────────────

pub fn init_window(
    window: &AppWindow,
    state: &Rc<RefCell<AppState>>,
    config_status: &str,
    db_path: &PathBuf,
) {
    let state = state.borrow();

    // Config view
    window.set_cfg_dest_root(
        state.config.destination_root.as_deref()
            .and_then(|p| p.to_str())
            .unwrap_or("")
            .into(),
    );
    window.set_cfg_sort_in_place(state.config.sort_in_place);
    window.set_cfg_case_insensitive(state.config.case_insensitive);
    window.set_cfg_extensions(state.config.extensions.join(", ").into());
    window.set_cfg_config_path(
        state.config_path.as_deref()
            .and_then(|p| p.to_str())
            .unwrap_or("(default)")
            .into(),
    );
    window.set_cfg_db_path(
        db_path.to_str().unwrap_or("(unknown)").into()
    );
    if !config_status.is_empty() {
        window.set_cfg_status(config_status.into());
    }

    // People view — populate list
    match state.store.list_people() {
        Ok(people) => window.set_people_list(models::people_to_model(&people)),
        Err(e) => window.set_people_status(format!("Failed to load people: {e}").into()),
    }
}

// ── Worker timer ──────────────────────────────────────────────────────────────

pub fn setup_timer(
    window: &AppWindow,
    state: Rc<RefCell<AppState>>,
    result_rx: mpsc::Receiver<JobResult>,
) -> slint::Timer {
    let weak = window.as_weak();
    let result_rx = Rc::new(RefCell::new(result_rx));
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(50),
        move || {
            let Some(win) = weak.upgrade() else { return };
            while let Ok(result) = result_rx.borrow_mut().try_recv() {
                let mut st = state.borrow_mut();
                match result {
                    JobResult::PlanReady(plan) => {
                        let summary = models::format_summary(&plan);
                        let plan_model = models::plan_to_model(&plan);
                        win.set_sort_summary(summary.into());
                        win.set_sort_plan_rows(plan_model);
                        win.set_sort_has_plan(true);
                        win.set_sort_busy(false);
                        win.set_sort_status("Plan ready — click Sort Now to proceed.".into());
                        st.current_plan = Some(plan);
                    }
                    JobResult::PlanFailed(err) => {
                        win.set_sort_busy(false);
                        win.set_sort_status(format!("Error building plan: {err}").into());
                    }
                    JobResult::ExecutionDone(report) => {
                        win.set_sort_busy(false);
                        win.set_sort_has_plan(false);
                        win.set_sort_plan_rows(models::empty_plan_model());
                        win.set_sort_summary("".into());
                        let status = if report.failed() > 0 {
                            format!(
                                "Done. Moved {} file(s). {} failed.",
                                report.moved(),
                                report.failed()
                            )
                        } else {
                            format!("Done. Moved {} file(s).", report.moved())
                        };
                        win.set_sort_status(status.into());
                    }
                }
            }
        },
    );
    timer
}

// ── Callback registration ─────────────────────────────────────────────────────

pub fn register_callbacks(
    window: &AppWindow,
    state: Rc<RefCell<AppState>>,
    job_tx: mpsc::Sender<Job>,
) {
    register_sort_callbacks(window, state.clone(), job_tx);
    register_people_callbacks(window, state.clone());
    register_config_callbacks(window, state);
}

// ── Sort callbacks ────────────────────────────────────────────────────────────

fn register_sort_callbacks(
    window: &AppWindow,
    state: Rc<RefCell<AppState>>,
    job_tx: mpsc::Sender<Job>,
) {
    // Pick source folder
    {
        let weak = window.as_weak();
        window.on_sort_pick_source(move || {
            if let Some(path) = dialogs::pick_folder() {
                if let Some(win) = weak.upgrade() {
                    win.set_sort_source(path.to_string_lossy().to_string().into());
                    win.set_sort_has_plan(false);
                    win.set_sort_summary("".into());
                    win.set_sort_plan_rows(models::empty_plan_model());
                    win.set_sort_status("".into());
                }
            }
        });
    }

    // Build plan
    {
        let weak = window.as_weak();
        let state2 = state.clone();
        let tx = job_tx.clone();
        window.on_sort_preview(move || {
            let Some(win) = weak.upgrade() else { return };
            let source_str = win.get_sort_source().to_string();
            if source_str.is_empty() {
                win.set_sort_status("Please pick a source folder first.".into());
                return;
            }
            let source_dir = PathBuf::from(&source_str);
            let st = state2.borrow();
            let config = st.config.clone();
            let alias_map = match st.store.load_alias_map(config.case_insensitive) {
                Ok(m) => m,
                Err(e) => {
                    win.set_sort_status(format!("Failed to load aliases: {e}").into());
                    return;
                }
            };
            let dest_root = match config.resolve_dest_root(&source_dir, None) {
                Ok(d) => d,
                Err(e) => {
                    win.set_sort_status(format!("Config error: {e}").into());
                    return;
                }
            };
            drop(st);
            win.set_sort_busy(true);
            win.set_sort_status("Building plan…".into());
            win.set_sort_has_plan(false);
            win.set_sort_summary("".into());
            win.set_sort_plan_rows(models::empty_plan_model());
            let _ = tx.send(Job::BuildPlan { source_dir, config, alias_map, dest_root });
        });
    }

    // Execute plan
    {
        let weak = window.as_weak();
        let tx = job_tx;
        window.on_sort_execute(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut st = state.borrow_mut();
            let Some(plan) = st.current_plan.take() else {
                win.set_sort_status("No plan available.".into());
                return;
            };
            win.set_sort_busy(true);
            win.set_sort_status("Sorting…".into());
            let _ = tx.send(Job::ExecutePlan { plan });
        });
    }
}

// ── People callbacks ──────────────────────────────────────────────────────────

fn refresh_people(win: &AppWindow, state: &AppState) {
    match state.store.list_people() {
        Ok(people) => win.set_people_list(models::people_to_model(&people)),
        Err(e) => win.set_people_status(format!("Error: {e}").into()),
    }
}

fn refresh_aliases(win: &AppWindow, state: &AppState, person_name: &str) {
    match state.store.list_aliases(Some(person_name)) {
        Ok(aliases) => win.set_alias_list(models::aliases_to_model(&aliases)),
        Err(e) => win.set_people_status(format!("Error: {e}").into()),
    }
}

fn register_people_callbacks(window: &AppWindow, state: Rc<RefCell<AppState>>) {
    // Select person
    {
        let weak = window.as_weak();
        let state2 = state.clone();
        window.on_people_select(move |idx| {
            let Some(win) = weak.upgrade() else { return };
            let st = state2.borrow();
            // Look up the name from the current list model
            let people_model = win.get_people_list();
            let count = people_model.row_count();
            if idx < 0 || idx as usize >= count {
                win.set_people_sel(-1);
                win.set_people_sel_name("".into());
                win.set_alias_list(models::aliases_to_model(&[]));
                return;
            }
            let row = people_model.row_data(idx as usize).unwrap_or_default();
            let name = row.name.to_string();
            win.set_people_sel(idx);
            win.set_people_sel_name(name.clone().into());
            refresh_aliases(&win, &st, &name);
        });
    }

    // Add person
    {
        let weak = window.as_weak();
        let state2 = state.clone();
        window.on_people_add(move |name, category| {
            let Some(win) = weak.upgrade() else { return };
            let st = state2.borrow();
            let name_str = name.trim().to_string();
            let cat_str = category.trim().to_string();
            let cat_opt = if cat_str.is_empty() { None } else { Some(cat_str.as_str()) };
            match st.store.add_person(&name_str, cat_opt) {
                Ok(_) => {
                    win.set_people_status(format!("Added '{name_str}'.").into());
                    refresh_people(&win, &st);
                }
                Err(e) => win.set_people_status(format!("Error: {e}").into()),
            }
        });
    }

    // Remove person
    {
        let weak = window.as_weak();
        let state2 = state.clone();
        window.on_people_remove(move |name| {
            let Some(win) = weak.upgrade() else { return };
            let st = state2.borrow();
            match st.store.remove_person(name.as_str()) {
                Ok(()) => {
                    win.set_people_status(format!("Removed '{name}'.").into());
                    win.set_people_sel(-1);
                    win.set_people_sel_name("".into());
                    win.set_alias_list(models::aliases_to_model(&[]));
                    refresh_people(&win, &st);
                }
                Err(e) => win.set_people_status(format!("Error: {e}").into()),
            }
        });
    }

    // Set category
    {
        let weak = window.as_weak();
        let state2 = state.clone();
        window.on_people_set_category(move |name, category| {
            let Some(win) = weak.upgrade() else { return };
            let st = state2.borrow();
            let cat_str = category.trim().to_string();
            let cat_opt = if cat_str.is_empty() { None } else { Some(cat_str.as_str()) };
            match st.store.set_category(name.as_str(), cat_opt) {
                Ok(()) => {
                    win.set_people_status("Category updated.".into());
                    refresh_people(&win, &st);
                }
                Err(e) => win.set_people_status(format!("Error: {e}").into()),
            }
        });
    }

    // Add alias
    {
        let weak = window.as_weak();
        let state2 = state.clone();
        window.on_alias_add(move |person, alias| {
            let Some(win) = weak.upgrade() else { return };
            let st = state2.borrow();
            let alias_str = alias.trim().to_string();
            match st.store.add_alias(person.as_str(), &alias_str) {
                Ok(()) => {
                    win.set_people_status(format!("Added alias '{alias_str}'.").into());
                    refresh_aliases(&win, &st, person.as_str());
                }
                Err(e) => win.set_people_status(format!("Error: {e}").into()),
            }
        });
    }

    // Remove alias
    {
        let weak = window.as_weak();
        let state2 = state.clone();
        window.on_alias_remove(move |alias| {
            let Some(win) = weak.upgrade() else { return };
            let st = state2.borrow();
            let sel_name = win.get_people_sel_name().to_string();
            match st.store.remove_alias(alias.as_str()) {
                Ok(()) => {
                    win.set_people_status(format!("Removed alias '{alias}'.").into());
                    if !sel_name.is_empty() {
                        refresh_aliases(&win, &st, &sel_name);
                    }
                }
                Err(e) => win.set_people_status(format!("Error: {e}").into()),
            }
        });
    }

    // Import CSV
    {
        let weak = window.as_weak();
        let state2 = state.clone();
        window.on_people_import_csv(move || {
            let Some(win) = weak.upgrade() else { return };
            let Some(path) = dialogs::pick_csv_for_import() else { return };
            let st = state2.borrow();
            match st.store.import_csv(&path) {
                Ok(result) => {
                    let mut msg = format!("Imported {} alias(es).", result.imported);
                    if result.skipped_duplicate > 0 {
                        msg.push_str(&format!(" Skipped {} duplicate(s).", result.skipped_duplicate));
                    }
                    if !result.errors.is_empty() {
                        msg.push_str(&format!(" {} error(s).", result.errors.len()));
                    }
                    win.set_people_status(msg.into());
                    refresh_people(&win, &st);
                    // Refresh aliases if a person is selected
                    let sel_name = win.get_people_sel_name().to_string();
                    if !sel_name.is_empty() {
                        refresh_aliases(&win, &st, &sel_name);
                    }
                }
                Err(e) => win.set_people_status(format!("Import failed: {e}").into()),
            }
        });
    }

    // Export CSV
    {
        let weak = window.as_weak();
        window.on_people_export_csv(move || {
            let Some(win) = weak.upgrade() else { return };
            let Some(path) = dialogs::pick_csv_for_save() else { return };
            let st = state.borrow();
            match st.store.export_csv(&path) {
                Ok(()) => win.set_people_status(
                    format!("Exported to '{}'.", path.display()).into()
                ),
                Err(e) => win.set_people_status(format!("Export failed: {e}").into()),
            }
        });
    }
}

// ── Config callbacks ──────────────────────────────────────────────────────────

fn config_from_window(win: &AppWindow) -> Config {
    let dest_str = win.get_cfg_dest_root().trim().to_string();
    let extensions: Vec<String> = win.get_cfg_extensions()
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    Config {
        destination_root: if dest_str.is_empty() { None } else { Some(PathBuf::from(dest_str)) },
        sort_in_place: win.get_cfg_sort_in_place(),
        case_insensitive: win.get_cfg_case_insensitive(),
        extensions,
        database: None,
    }
}

fn register_config_callbacks(window: &AppWindow, state: Rc<RefCell<AppState>>) {
    // Pick destination folder
    {
        let weak = window.as_weak();
        window.on_cfg_pick_dest(move || {
            if let (Some(path), Some(win)) = (dialogs::pick_folder(), weak.upgrade()) {
                win.set_cfg_dest_root(path.to_string_lossy().to_string().into());
            }
        });
    }

    // Save
    {
        let weak = window.as_weak();
        let state2 = state.clone();
        window.on_cfg_save(move || {
            let Some(win) = weak.upgrade() else { return };
            let new_config = config_from_window(&win);
            if let Err(e) = new_config.validate() {
                win.set_cfg_status(format!("Validation failed: {e}").into());
                return;
            }
            let path = match state2.borrow().config_path.clone() {
                Some(p) => p,
                None => match Config::default_path() {
                    Some(p) => p,
                    None => {
                        win.set_cfg_status("Cannot determine config path.".into());
                        return;
                    }
                },
            };
            match new_config.save(&path) {
                Ok(()) => {
                    win.set_cfg_config_path(path.to_string_lossy().to_string().into());
                    win.set_cfg_status(format!("Saved to '{}'.", path.display()).into());
                    state2.borrow_mut().config = new_config;
                }
                Err(e) => win.set_cfg_status(format!("Save failed: {e}").into()),
            }
        });
    }

    // Reload
    {
        let weak = window.as_weak();
        let state2 = state.clone();
        window.on_cfg_reload(move || {
            let Some(win) = weak.upgrade() else { return };
            let path = match state2.borrow().config_path.clone() {
                Some(p) => p,
                None => {
                    win.set_cfg_status("No config path set.".into());
                    return;
                }
            };
            match Config::load(&path) {
                Ok(cfg) => {
                    win.set_cfg_dest_root(
                        cfg.destination_root.as_deref()
                            .and_then(|p| p.to_str()).unwrap_or("").into(),
                    );
                    win.set_cfg_sort_in_place(cfg.sort_in_place);
                    win.set_cfg_case_insensitive(cfg.case_insensitive);
                    win.set_cfg_extensions(cfg.extensions.join(", ").into());
                    win.set_cfg_status("Config reloaded.".into());
                    // Reopen store if db path changed
                    let db_path = cfg.resolved_db_path()
                        .or_else(Config::default_db_path);
                    if let Some(db_path) = db_path {
                        match Store::open(&db_path) {
                            Ok(store) => {
                                let mut st = state2.borrow_mut();
                                st.store = store;
                                st.config = cfg;
                            }
                            Err(e) => {
                                win.set_cfg_status(
                                    format!("Config reloaded but database error: {e}").into()
                                );
                            }
                        }
                    } else {
                        state2.borrow_mut().config = cfg;
                    }
                }
                Err(e) => win.set_cfg_status(format!("Reload failed: {e}").into()),
            }
        });
    }

    // Validate
    {
        let weak = window.as_weak();
        window.on_cfg_validate(move || {
            let Some(win) = weak.upgrade() else { return };
            let cfg = config_from_window(&win);
            match cfg.validate() {
                Ok(()) => win.set_cfg_status("Config OK.".into()),
                Err(e) => win.set_cfg_status(format!("Validation failed: {e}").into()),
            }
        });
    }
}
