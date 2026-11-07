//! The global `hooks` from `config.toml`: host commands run around `up` and
//! `destroy`.

use std::path::Path;

use crate::config::Hooks;
use crate::devcontainer::lifecycle_command::LifecycleCommand;
use crate::workspace::Workspace;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Hook {
    BeforeUp,
    AfterUp,
    BeforeDestroy,
    AfterDestroy,
}

impl Hook {
    /// The config property, used to name the command in output and errors.
    fn field(self) -> &'static str {
        match self {
            Hook::BeforeUp => "hooks.beforeUp",
            Hook::AfterUp => "hooks.afterUp",
            Hook::BeforeDestroy => "hooks.beforeDestroy",
            Hook::AfterDestroy => "hooks.afterDestroy",
        }
    }

    fn command(self, hooks: &Hooks) -> Option<&LifecycleCommand> {
        match self {
            Hook::BeforeUp => hooks.before_up.as_ref(),
            Hook::AfterUp => hooks.after_up.as_ref(),
            Hook::BeforeDestroy => hooks.before_destroy.as_ref(),
            Hook::AfterDestroy => hooks.after_destroy.as_ref(),
        }
    }

    /// Where the hook runs: the workspace while it exists, else the project root.
    fn dir<'a>(self, workspace: &'a Workspace<'_>) -> &'a Path {
        match self {
            Hook::BeforeUp | Hook::AfterDestroy => &workspace.state.project.path,
            Hook::AfterUp | Hook::BeforeDestroy => &workspace.path,
        }
    }
}

/// Run `hook` for `workspace`, if it is configured.
pub(crate) async fn run(hooks: &Hooks, hook: Hook, workspace: &Workspace<'_>) -> eyre::Result<()> {
    let Some(cmd) = hook.command(hooks) else {
        return Ok(());
    };
    let env = HookEnv::new(
        &workspace.state.project_name,
        &workspace.state.project.path,
        &workspace.name,
        &workspace.path,
        workspace.is_root,
    );
    cmd.run_on_host(hook.field(), Some(hook.dir(workspace)), &env.pairs())
        .await
}

/// The environment variables a hook gets.
struct HookEnv {
    vars: [(&'static str, String); 5],
}

impl HookEnv {
    fn new(
        project: &str,
        project_path: &Path,
        workspace: &str,
        workspace_path: &Path,
        is_root: bool,
    ) -> Self {
        Self {
            vars: [
                ("DEVCONCURRENT_PROJECT", project.to_owned()),
                (
                    "DEVCONCURRENT_PROJECT_PATH",
                    project_path.display().to_string(),
                ),
                ("DEVCONCURRENT_WORKSPACE", workspace.to_owned()),
                (
                    "DEVCONCURRENT_WORKSPACE_PATH",
                    workspace_path.display().to_string(),
                ),
                ("DEVCONCURRENT_ROOT", is_root.to_string()),
            ],
        }
    }

    fn pairs(&self) -> Vec<(&str, &str)> {
        self.vars.iter().map(|(k, v)| (*k, v.as_str())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hooks_get_their_env_and_dir() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        let env = HookEnv::new("proj", Path::new("/p"), "ws", Path::new("/p/ws"), false);
        let cmd: LifecycleCommand = serde_json::from_str(
            r#""printf '%s %s %s %s %s %s' \"$DEVCONCURRENT_PROJECT\" \"$DEVCONCURRENT_PROJECT_PATH\" \"$DEVCONCURRENT_WORKSPACE\" \"$DEVCONCURRENT_WORKSPACE_PATH\" \"$DEVCONCURRENT_ROOT\" \"$(pwd -P)\" > out""#,
        )
        .unwrap();
        cmd.run_on_host("hooks.test", Some(dir.path()), &env.pairs())
            .await
            .unwrap();

        let pwd = dir.path().canonicalize().unwrap();
        assert_eq!(
            std::fs::read_to_string(out).unwrap(),
            format!("proj /p ws /p/ws false {}", pwd.display()),
        );
    }
}
