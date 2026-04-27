use std::{env, path::PathBuf};

use eyre::OptionExt;

use crate::{
    config::{Config, Project},
    devcontainer::{self, DevcontainerConfig, dc_options::DcOptions},
    docker::DockerClient,
    workspace::WorkspaceMini,
    worktree,
};

pub struct State {
    pub project_name: String,
    pub project: Project,
    pub devcontainer: Option<DevcontainerState>,
}

pub struct DevcontainerState {
    pub path: PathBuf,
    pub config: DevcontainerConfig,
    pub docker: DockerClient,
}

impl DevcontainerState {
    async fn new(project: &Project) -> eyre::Result<Option<Self>> {
        let Some(path) = DevcontainerConfig::find_config(&project.path) else {
            return Ok(None);
        };
        let config = DevcontainerConfig::load(&path)?;
        let docker = DockerClient::new().await?;

        Ok(Some(Self {
            path,
            config,
            docker,
        }))
    }

    pub fn compose(&self) -> &devcontainer::Compose {
        let crate::devcontainer::Kind::Compose(ref compose) = self.config.kind else {
            unimplemented!();
        };
        compose
    }

    pub fn devconcurrent(&self) -> &DcOptions {
        &self.config.common.customizations.devconcurrent
    }
}

impl State {
    pub async fn new(specified_project: Option<String>) -> eyre::Result<Self> {
        let config = Config::load()?;
        let (project_name, project) = config.project(specified_project)?;

        let devcontainer = DevcontainerState::new(&project).await?;

        Ok(Self {
            project_name,
            project,
            devcontainer,
        })
    }

    pub fn is_root(&self, name: &str) -> bool {
        self.project
            .path
            .file_name()
            .is_some_and(|root| name == root)
    }

    /// The directory we have to create git worktrees and docker override files
    ///
    /// In priority:
    ///
    /// * Read from devconcurrent config file for the project
    /// * Read from customizations.devconcurrent in devcontainer.json
    /// * Defaults to /tmp/devconcurrent/<PROJECT_NAME>/
    pub fn project_working_dir(&self) -> PathBuf {
        let dir = self
            .project
            .worktree_folder
            .clone()
            .or_else(|| {
                self.devcontainer.as_ref().and_then(|dc| {
                    dc.config
                        .common
                        .customizations
                        .devconcurrent
                        .worktree_folder
                        .clone()
                })
            })
            .unwrap_or_else(|| PathBuf::from_iter(["/tmp", "devconcurrent", &self.project_name]));

        if dir.is_relative() {
            self.project.path.join(dir)
        } else {
            dir
        }
    }

    fn worktree_path(&self, workspace_name: &str) -> PathBuf {
        self.project_working_dir().join(workspace_name)
    }

    /// Find the workspace name.
    ///
    /// If no name is given, or if it's ".", we derive it from the current working direcory.
    pub async fn resolve_workspace(&self, name: Option<String>) -> eyre::Result<WorkspaceMini> {
        let worktrees = worktree::list(&self.project.path).await?;

        if let Some(workspace_name) = name
            && workspace_name != "."
        {
            let path = worktrees
                .into_iter()
                .find(|wt| wt.file_name() == Some(workspace_name.as_ref()))
                .unwrap_or_else(|| self.worktree_path(&workspace_name));
            let root = self.is_root(&workspace_name);
            return Ok(WorkspaceMini {
                name: workspace_name,
                path,
                root,
            });
        }

        let cwd = env::current_dir()?;

        let path = worktrees
            .into_iter()
            .filter(|wt| cwd.starts_with(wt))
            .max_by_key(|wt| wt.as_os_str().len())
            .ok_or_else(|| {
                eyre::eyre!(
                    "no workspace specified and not inside a worktree of project '{}'",
                    self.project_name
                )
            })?;

        let name = path
            .file_name()
            .ok_or_eyre("worktree path has no basename")?
            .to_string_lossy()
            .to_string();

        let root = self.is_root(&name);

        Ok(WorkspaceMini { name, path, root })
    }

    pub fn try_devcontainer(&self) -> eyre::Result<&DevcontainerState> {
        self.devcontainer.as_ref().ok_or_else(|| eyre::eyre!("no devcontainer.json found for this project; devcontainer functionality is disabled"))
    }
}
