use std::path::PathBuf;

use bollard::Docker;
use bollard::secret::ContainerSummaryStateEnum;
use clap::Args;
use eyre::eyre;
use nucleo_picker::{Picker, Render};

use crate::config::Config;
use crate::devcontainer::DevContainer;
use crate::workspace::{PickerItem, Workspace, picker_items};

/// Exec into a running devcontainer
///
/// Supply either project or name, or leave both blank to get a picker.
#[derive(Debug, Args)]
#[command(verbatim_doc_comment)]
pub struct Exec {
    #[arg(short, long, conflicts_with = "name")]
    project: Option<String>,

    #[arg(short, long, conflicts_with = "project")]
    name: Option<String>,

    #[arg(
        num_args = 0..,
        allow_hyphen_values = true,
        trailing_var_arg = true,
    )]
    cmd: Vec<String>,
}

struct PickerItemRenderer;

impl Render<PickerItem> for PickerItemRenderer {
    type Str<'a> = &'a str;

    fn render<'a>(&self, item: &'a PickerItem) -> Self::Str<'a> {
        &item.rendered
    }
}

impl Exec {
    pub async fn run(self, docker: &Docker, config: &Config) -> eyre::Result<()> {
        let (path, container_id) = if let Some(ref name) = self.name {
            let workspaces = Workspace::list_project(docker, None, config).await?;
            let ws = workspaces
                .into_iter()
                .find(|ws| {
                    ws.path
                        .file_name()
                        .map(|f| f == name.as_str())
                        .unwrap_or(false)
                })
                .ok_or_else(|| eyre!("no workspace found with name: {name}"))?;
            if ws.status != ContainerSummaryStateEnum::RUNNING {
                return Err(eyre!("workspace is not running: {}", ws.path.display()));
            }
            let cid = ws
                .container_ids
                .into_iter()
                .next()
                .ok_or_else(|| eyre!("no containers for workspace"))?;
            (ws.path, cid)
        } else {
            let mut workspaces =
                Workspace::list_project(docker, self.project.as_deref(), config).await?;
            workspaces.retain(|ws| ws.status == ContainerSummaryStateEnum::RUNNING);
            pick_workspace(workspaces)?
        };

        let dc = DevContainer::load(&path)?;
        let crate::devcontainer::Kind::Compose(ref compose) = dc.kind else {
            panic!();
        };

        super::up::exec_interactive(
            &container_id,
            dc.common.remote_user.as_deref(),
            Some(compose.workspace_folder.as_path()),
            &self.cmd,
            config,
        )
    }
}

fn pick_workspace(workspaces: Vec<Workspace>) -> eyre::Result<(PathBuf, String)> {
    match workspaces.len() {
        0 => Err(eyre!("no running workspaces found")),
        1 => {
            let ws = workspaces.into_iter().next().unwrap();
            let cid = ws
                .container_ids
                .into_iter()
                .next()
                .ok_or_else(|| eyre!("no containers for workspace"))?;
            Ok((ws.path, cid))
        }
        _ => {
            let items = picker_items(workspaces);
            let mut picker = Picker::new(PickerItemRenderer);
            let injector = picker.injector();
            for item in items {
                injector.push(item);
            }
            let item = picker
                .pick()
                .map_err(|e| eyre!("{e}"))?
                .ok_or_else(|| eyre!("no workspace selected"))?;
            let cid = item
                .workspace
                .container_ids
                .first()
                .cloned()
                .ok_or_else(|| eyre!("no containers for workspace"))?;
            Ok((item.workspace.path.clone(), cid))
        }
    }
}
