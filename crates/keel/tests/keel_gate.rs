//! Keel exit gate: CLI parsing + the cache command's backend selection.

use clap::Parser;

use keel::cli::{CacheAction, CacheBackend, Cli, Command, WorkspaceAction};

#[test]
fn parses_cache_stats_with_explicit_backend() {
    let cli = Cli::try_parse_from(["keel", "cache", "stats", "--cache-backend", "memory"]).unwrap();
    assert_eq!(cli.cache_backend, CacheBackend::Memory);
    match cli.command {
        Command::Cache { action } => assert!(matches!(action, CacheAction::Stats)),
        _ => panic!("expected the cache command"),
    }
}

#[test]
fn default_cache_backend_is_auto() {
    let cli = Cli::try_parse_from(["keel", "cache", "path"]).unwrap();
    assert_eq!(cli.cache_backend, CacheBackend::Auto);
}

#[test]
fn parses_add_with_name() {
    let cli = Cli::try_parse_from(["keel", "add", "serde"]).unwrap();
    match cli.command {
        Command::Add { name, path } => {
            assert_eq!(name, "serde");
            assert!(path.is_none());
        }
        _ => panic!("expected add"),
    }
}

#[test]
fn parses_workspace_member_selection() {
    let cli = Cli::try_parse_from(["keel", "workspace", "build", "app"]).unwrap();
    match cli.command {
        Command::Workspace {
            action: Some(WorkspaceAction::Build { package }),
        } => assert_eq!(package.as_deref(), Some("app")),
        _ => panic!("expected workspace build"),
    }
}

#[test]
fn cache_dir_points_at_keel() {
    let d = keel::cache_dir();
    let s = d.display().to_string();
    assert!(
        d.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("keel")),
        "cache dir ends with 'keel' (case-insensitive on Windows): {s}"
    );
}
