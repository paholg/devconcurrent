use std::fs;
use std::path::PathBuf;

use eyre::eyre;

fn archive_dir(project_name: &str) -> eyre::Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "devconcurrent")
        .ok_or_else(|| eyre!("could not determine data directory"))?;
    Ok(dirs.data_dir().join("archived").join(project_name))
}

pub struct ArchivedWorkspace {
    pub compose_project: String,
    pub workspace_name: String,
}

pub fn archive(
    project_name: &str,
    compose_project: &str,
    workspace_name: &str,
) -> eyre::Result<()> {
    let dir = archive_dir(project_name)?;
    fs::create_dir_all(&dir)?;
    fs::write(dir.join(compose_project), workspace_name)?;
    Ok(())
}

pub fn unarchive(project_name: &str, compose_project: &str) -> eyre::Result<()> {
    let dir = archive_dir(project_name)?;
    let path = dir.join(compose_project);
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

/// Find the oldest archived workspace for a project (by file mtime).
pub fn find_archived(project_name: &str) -> eyre::Result<Option<ArchivedWorkspace>> {
    let dir = match archive_dir(project_name) {
        Ok(d) => d,
        Err(_) => return Ok(None),
    };
    if !dir.exists() {
        return Ok(None);
    }

    let mut oldest: Option<(std::time::SystemTime, ArchivedWorkspace)> = None;

    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let compose_project = entry.file_name().to_string_lossy().to_string();
        let workspace_name = fs::read_to_string(entry.path())
            .unwrap_or_default()
            .trim()
            .to_string();
        if workspace_name.is_empty() {
            continue;
        }
        let mtime = entry
            .metadata()?
            .modified()
            .unwrap_or(std::time::UNIX_EPOCH);
        if oldest.as_ref().is_none_or(|(t, _)| mtime < *t) {
            oldest = Some((
                mtime,
                ArchivedWorkspace {
                    compose_project,
                    workspace_name,
                },
            ));
        }
    }

    Ok(oldest.map(|(_, aw)| aw))
}

pub fn is_archived(project_name: &str, compose_project: &str) -> bool {
    archive_dir(project_name)
        .map(|dir| dir.join(compose_project).exists())
        .unwrap_or(false)
}

/// List all archived workspaces for a project.
pub fn list_archived(project_name: &str) -> Vec<ArchivedWorkspace> {
    let dir = match archive_dir(project_name) {
        Ok(d) if d.exists() => d,
        _ => return Vec::new(),
    };

    let mut result = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let compose_project = entry.file_name().to_string_lossy().to_string();
            let workspace_name = fs::read_to_string(entry.path())
                .unwrap_or_default()
                .trim()
                .to_string();
            if !workspace_name.is_empty() {
                result.push(ArchivedWorkspace {
                    compose_project,
                    workspace_name,
                });
            }
        }
    }
    result
}
