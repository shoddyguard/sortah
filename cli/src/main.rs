use anyhow::{Context, Result};
use clap::Parser;
use sortah_core::{Config, Store};
use std::io::{self, Write};
use std::path::PathBuf;

mod cli;
use cli::{AliasCommand, Cli, Command, ConfigCommand, PersonCommand};

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Config(cmd) => handle_config(cmd, cli.config),
        Command::Sort(args) => handle_sort(args, cli.config),
        Command::Person(cmd) => handle_person(cmd, cli.config),
        Command::Alias(cmd) => handle_alias(cmd, cli.config),
        Command::List(args) => handle_list(args, cli.config),
        Command::Import(args) => handle_import(args, cli.config),
        Command::Export(args) => handle_export(args, cli.config),
    }
}

// ---- Helpers ----

fn load_config(config_path: Option<PathBuf>) -> Result<Config> {
    let path = config_path
        .or_else(Config::default_path)
        .context("Cannot determine config path; use --config or set SORTAH_CONFIG")?;
    Config::load(&path).with_context(|| format!("Failed to load config from '{}'", path.display()))
}

fn open_store(config: &Config) -> Result<Store> {
    let db_path = config
        .resolved_db_path()
        .context("Cannot determine database path")?;
    Store::open(&db_path)
        .with_context(|| format!("Failed to open database at '{}'", db_path.display()))
}

// ---- Handlers ----

fn handle_config(cmd: ConfigCommand, config_path: Option<PathBuf>) -> Result<()> {
    match cmd {
        ConfigCommand::Init => {
            let path = config_path
                .or_else(Config::default_path)
                .context("Cannot determine config path; use --config")?;

            if path.exists() {
                println!("Config already exists at '{}'", path.display());
                println!("Edit it directly or delete it and re-run 'sortah config init'.");
            } else {
                Config::write_template(&path)?;
                println!("Config written to '{}'", path.display());
                println!("Edit it before running 'sortah sort'.");
            }

            let db_path = Config::default_db_path()
                .context("Cannot determine database path")?;
            Store::open(&db_path)?;
            println!("Database ready at '{}'", db_path.display());
        }

        ConfigCommand::Path => {
            let config_path = config_path.or_else(Config::default_path);
            println!(
                "Config:   {}",
                config_path
                    .as_deref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(unknown)".into())
            );
            let db_display = config_path
                .as_deref()
                .and_then(|p| Config::load(p).ok())
                .and_then(|c| c.resolved_db_path())
                .or_else(Config::default_db_path)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "(unknown)".into());
            println!("Database: {}", db_display);
        }

        ConfigCommand::Validate => {
            let config = load_config(config_path)?;
            println!("Config OK");
            println!("  destination_root: {}", config.destination_root.display());
            println!("  case_insensitive: {}", config.case_insensitive);
            println!("  extensions:       {}", config.extensions.join(", "));

            let store = open_store(&config)?;
            let people = store.list_people()?;
            let aliases = store.list_aliases(None)?;
            let with_category = people.iter().filter(|p| p.category.is_some()).count();
            println!("Database OK");
            println!("  people:           {}", people.len());
            println!("  with category:    {}", with_category);
            println!("  aliases:          {}", aliases.len());

            if config.case_insensitive {
                let collisions = store.find_case_collisions()?;
                if !collisions.is_empty() {
                    println!();
                    println!(
                        "Warning: {} case-insensitive collision(s) detected.",
                        collisions.len()
                    );
                    println!("These pairs differ only by case and may match ambiguously:");
                    for c in &collisions {
                        println!(
                            "  '{}' ({}) vs '{}' ({})",
                            c.alias_a, c.name_a, c.alias_b, c.name_b
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

fn handle_sort(args: cli::SortArgs, config_path: Option<PathBuf>) -> Result<()> {
    use sortah_core::engine::{build_plan, execute_plan};
    use sortah_core::report::PlannedAction;

    let config = load_config(config_path)?;
    let store = open_store(&config)?;
    let alias_map = store.load_alias_map(config.case_insensitive)?;

    let cwd = std::env::current_dir().context("Cannot determine current directory")?;
    let dest_override = args.dest.as_deref();

    let plan =
        build_plan(&cwd, &config, &alias_map, dest_override).context("Failed to build plan")?;
    let summary = plan.summary();

    println!("Sorting: {}", cwd.display());
    println!();
    println!("  Files to move:     {}", summary.to_move);
    if summary.skip_duplicate > 0 {
        println!("  Skip (duplicate):  {}", summary.skip_duplicate);
    }
    if summary.unknown_total() > 0 {
        println!("  Skip (unknown):    {}", summary.unknown_total());
    }

    if !summary.by_person.is_empty() {
        println!();
        println!("  By person:");
        let mut by_person: Vec<_> = summary.by_person.iter().collect();
        by_person.sort_by_key(|(name, _)| name.as_str());
        for (person, count) in by_person {
            println!("    {}: {} file(s)", person, count);
        }
    }

    if !summary.unknown_usernames.is_empty() {
        println!();
        println!("  Unknown usernames (files left in place):");
        for (username, count) in &summary.unknown_usernames {
            println!("    {} ({} file(s))", username, count);
        }
    }

    if args.verbose {
        println!();
        println!("  Planned moves:");
        for action in &plan.actions {
            if let PlannedAction::Move(m) = action {
                println!("    {} -> {}", m.src.display(), m.dst.display());
            }
        }
    }

    println!();

    if summary.to_move == 0 {
        println!("Nothing to move.");
        return Ok(());
    }

    let proceed = if args.yes {
        true
    } else {
        print!("Proceed? [y/N] ");
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
    };

    if !proceed {
        println!("Aborted.");
        return Ok(());
    }

    let report = execute_plan(&plan);

    println!("Done. Moved {} file(s).", report.moved());
    if report.failed() > 0 {
        eprintln!();
        eprintln!("{} file(s) failed:", report.failed());
        for (src, err) in report.failures() {
            eprintln!("  {}: {}", src.display(), err);
        }
        std::process::exit(1);
    }

    Ok(())
}

fn handle_person(cmd: PersonCommand, config_path: Option<PathBuf>) -> Result<()> {
    let config = load_config(config_path)?;
    let store = open_store(&config)?;
    match cmd {
        PersonCommand::Add { name, category } => {
            store.add_person(&name, category.as_deref())?;
            match &category {
                Some(cat) => println!("Added person: {} [{}]", name, cat),
                None => println!("Added person: {} [Uncategorised]", name),
            }
        }
        PersonCommand::Rm { name } => {
            store.remove_person(&name)?;
            println!("Removed: {}", name);
        }
        PersonCommand::SetCategory { name, category } => {
            store.set_category(&name, category.as_deref())?;
            match &category {
                Some(cat) => println!("Set category for '{}' to '{}'", name, cat),
                None => println!("Cleared category for '{}'", name),
            }
        }
    }
    Ok(())
}

fn handle_alias(cmd: AliasCommand, config_path: Option<PathBuf>) -> Result<()> {
    let config = load_config(config_path)?;
    let store = open_store(&config)?;
    match cmd {
        AliasCommand::Add { name, alias } => {
            store.add_alias(&name, &alias)?;
            println!("Added alias '{}' for '{}'", alias, name);
        }
        AliasCommand::Rm { alias } => {
            store.remove_alias(&alias)?;
            println!("Removed alias '{}'", alias);
        }
    }
    Ok(())
}

fn handle_list(args: cli::ListArgs, config_path: Option<PathBuf>) -> Result<()> {
    let config = load_config(config_path)?;
    let store = open_store(&config)?;

    if let Some(ref person) = args.person {
        let p = store.get_person(person)?;
        let cat = p.category.as_deref().unwrap_or("Uncategorised");
        println!("{} [{}]:", person, cat);
        let aliases = store.list_aliases(Some(person))?;
        if aliases.is_empty() {
            println!("  (no aliases)");
        } else {
            for a in &aliases {
                println!("  {}", a.alias);
            }
        }
    } else {
        let people = store.list_people()?;
        if people.is_empty() {
            println!("No people configured.");
            println!("Use 'sortah person add <name>' or 'sortah import <file.csv>'.");
            return Ok(());
        }
        for person in &people {
            let cat = person.category.as_deref().unwrap_or("Uncategorised");
            println!("{} [{}]", person.name, cat);
            let aliases = store.list_aliases(Some(&person.name))?;
            if aliases.is_empty() {
                println!("  (no aliases)");
            } else {
                for a in &aliases {
                    println!("  {}", a.alias);
                }
            }
        }
    }
    Ok(())
}

fn handle_import(args: cli::ImportArgs, config_path: Option<PathBuf>) -> Result<()> {
    let config = load_config(config_path)?;
    let store = open_store(&config)?;

    let result = store
        .import_csv(&args.file)
        .with_context(|| format!("Failed to import '{}'", args.file.display()))?;

    println!("Imported {} alias(es).", result.imported);
    if result.skipped_duplicate > 0 {
        println!("Skipped {} duplicate(s).", result.skipped_duplicate);
    }
    if !result.errors.is_empty() {
        eprintln!("{} error(s):", result.errors.len());
        for e in &result.errors {
            eprintln!("  {}", e);
        }
    }
    Ok(())
}

fn handle_export(args: cli::ExportArgs, config_path: Option<PathBuf>) -> Result<()> {
    let config = load_config(config_path)?;
    let store = open_store(&config)?;

    store
        .export_csv(&args.file)
        .with_context(|| format!("Failed to export to '{}'", args.file.display()))?;

    println!("Exported to '{}'", args.file.display());
    Ok(())
}
