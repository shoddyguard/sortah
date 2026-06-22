use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "sortah",
    version,
    about = "Sort downloaded images into per-person directories by username"
)]
pub struct Cli {
    /// Path to the config file (overrides the platform default and SORTAH_CONFIG)
    #[arg(long, short = 'c', global = true, env = "SORTAH_CONFIG")]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Sort image files in the current directory into per-person directories
    Sort(SortArgs),

    /// Manage configuration
    #[command(subcommand)]
    Config(ConfigCommand),

    /// Manage people (destination directories)
    #[command(subcommand)]
    Person(PersonCommand),

    /// Manage username aliases
    #[command(subcommand)]
    Alias(AliasCommand),

    /// List people and their aliases
    List(ListArgs),

    /// Bulk-import aliases from a CSV file (headers: category,name,alias or name,alias)
    Import(ImportArgs),

    /// Export all aliases to a CSV file
    Export(ExportArgs),
}

#[derive(Args, Debug)]
pub struct SortArgs {
    /// Skip the confirmation prompt and proceed immediately
    #[arg(long, short = 'y', alias = "confirm")]
    pub yes: bool,

    /// Override destination_root from config
    #[arg(long)]
    pub dest: Option<PathBuf>,

    /// Print each planned move in the summary
    #[arg(long, short = 'v')]
    pub verbose: bool,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// Write a starter config file and create an empty database
    Init,
    /// Print the resolved config and database paths
    Path,
    /// Validate config, report people/alias counts and any case collisions
    Validate,
}

#[derive(Subcommand, Debug)]
pub enum PersonCommand {
    /// Add a new person
    Add {
        /// The name directory name for this person
        name: String,
        /// Category folder this person's images are sorted into
        #[arg(long)]
        category: Option<String>,
    },
    /// Remove a person and all their aliases
    Rm {
        /// The name name of the person to remove
        name: String,
    },
    /// Set (or clear) the category for a person
    SetCategory {
        /// The name of the person
        name: String,
        /// New category value (omit to clear)
        category: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum AliasCommand {
    /// Add a username alias for a person
    Add {
        /// The name name of the person
        name: String,
        /// The alias to add (stored verbatim)
        alias: String,
    },
    /// Remove a username alias
    Rm {
        /// The alias to remove
        alias: String,
    },
}

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Show aliases only for this person
    #[arg(long)]
    pub person: Option<String>,
}

#[derive(Args, Debug)]
pub struct ImportArgs {
    /// CSV file to import (header: category,name,alias... or name,alias...)
    pub file: PathBuf,
}

#[derive(Args, Debug)]
pub struct ExportArgs {
    /// Output CSV file path
    pub file: PathBuf,
}
