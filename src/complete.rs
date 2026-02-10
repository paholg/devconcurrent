use std::ffi::OsStr;
use std::path::PathBuf;

use clap_complete::engine::CompletionCandidate;

use crate::config::Config;
use crate::worktree;

pub fn complete_workspace(current: &OsStr) -> Vec<CompletionCandidate> {
    let prefix = current.to_string_lossy();
    let Ok(config) = Config::load() else {
        return vec![];
    };
    worktree_basenames(config)
        .into_iter()
        .filter(|name| name.starts_with(prefix.as_ref()))
        .map(CompletionCandidate::new)
        .collect()
}

pub fn complete_project(current: &OsStr) -> Vec<CompletionCandidate> {
    let prefix = current.to_string_lossy();
    let Ok(config) = Config::load() else {
        return vec![];
    };
    config
        .projects
        .keys()
        .filter(|name| name.starts_with(prefix.as_ref()))
        .map(CompletionCandidate::new)
        .collect()
}

fn resolve_project_path(config: Config) -> Option<PathBuf> {
    let project_name = project_from_args().or_else(|| std::env::var("DC_PROJECT").ok());
    let (_name, project) = config.project(project_name.as_deref()).ok()?;
    Some(project.path)
}

// TODO: See if we can extract this from our existing clap setup instead.
fn project_from_args() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    // Look only after the `--` separator.
    let start = args.iter().position(|a| a == "--")? + 1;
    let completion_args = &args[start..];
    let mut iter = completion_args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--project" || arg == "-p" {
            return iter.next().cloned();
        }
        if let Some(val) = arg.strip_prefix("--project=") {
            return Some(val.to_string());
        }
    }
    None
}

fn worktree_basenames(config: Config) -> Vec<String> {
    let Some(path) = resolve_project_path(config) else {
        return vec![];
    };
    worktree::list_sync(&path)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|path| path.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect()
}
