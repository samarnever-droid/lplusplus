//! The Keel command surface (clap derive).
//!
//! A strong, full-lifecycle vocabulary (the Cargo-analog): new/init,
//! add/remove, fetch/search/publish, build/check/run/test, cache.

use clap::{Parser, Subcommand, ValueEnum};

/// Keel — the L++ build & package manager.
#[derive(Parser)]
#[command(name = "keel", version, about)]
pub struct Cli {
    /// Hot cache backend for the resolver/index (global to all subcommands).
    #[arg(long, value_enum, global = true, default_value_t = CacheBackend::Auto)]
    pub cache_backend: CacheBackend,

    /// The git-registry URL (overrides the `KEEL_REGISTRY` env var).
    #[arg(long, global = true)]
    pub registry: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

/// Which hot cache backend to use. `auto` = the safe in-memory default.
#[derive(ValueEnum, Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum CacheBackend {
    #[default]
    Auto,
    Memory,
}

#[derive(Subcommand)]
pub enum Command {
    /// Create a new L++ project
    New {
        /// Project name
        name: String,
    },
    /// Initialize a project in the current directory
    Init,
    /// Add a dependency
    Add {
        /// Package name (or `name@version`)
        name: String,
    },
    /// Remove a dependency
    Remove {
        /// Package name
        name: String,
    },
    /// Compile the current workspace (native + wasm)
    Build,
    /// Build and run the current project
    Run,
    /// Run the test suite
    Test,
    /// Fast type-check, no codegen
    Check,
    /// Fetch dependencies into the global cache
    Fetch {
        /// Package to fetch (or `name@version`); omit to fetch all (resolver)
        name: Option<String>,
    },
    /// Search the registry
    Search {
        /// Query
        query: String,
    },
    /// Publish a package to the registry
    Publish,
    /// Update `Keel.lock` to the latest compatible versions (or just one package)
    Update {
        /// Update only this package; every other locked package stays pinned
        package: Option<String>,
    },
    /// Show the resolved dependency tree of each workspace member
    Tree {
        /// Only show this member's tree
        #[arg(short, long)]
        package: Option<String>,
    },
    /// Show which locked packages have newer versions in the registry
    Outdated {
        /// Only report on this package
        package: Option<String>,
    },
    /// Explain why a package is in the resolved graph (its dependency chains)
    Why {
        /// Package name
        name: String,
    },
    /// Verify the integrity of every locked registry artifact (supply-chain audit)
    Verify,
    /// Inspect or clean the global cache
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
}

#[derive(Subcommand)]
pub enum CacheAction {
    /// Show cache statistics as a table
    Stats,
    /// Remove cached artifacts
    Clean,
    /// Print the global cache directory
    Path,
}
