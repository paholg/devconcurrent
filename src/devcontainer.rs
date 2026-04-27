use std::path::{Path, PathBuf};

use eyre::WrapErr;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_inline_default::serde_inline_default;
use serde_with::{OneOrMany, serde_as};

pub(crate) mod dc_options;
pub(crate) mod forward_port;
pub(crate) mod lifecycle_command;
mod unsupported;

use crate::devcontainer::{dc_options::DcOptions, forward_port::ForwardPort};
use lifecycle_command::LifecycleCommand;
use unsupported::Unsupported;

/// Devcontainer config from devcontainer.json.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub(crate) struct DevcontainerConfig {
    #[serde(flatten)]
    pub(crate) common: Common,
    #[serde(flatten)]
    pub(crate) kind: Kind,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(untagged)]
pub(crate) enum Kind {
    Compose(Compose),
    #[serde(deserialize_with = "unsupported::Image::error")]
    Image(Image),
    #[serde(deserialize_with = "unsupported::Dockerfile::error")]
    Dockerfile(Box<Dockerfile>),
}

impl DevcontainerConfig {
    /// Find the appropriate devcontainer.json file from the given root directory.
    ///
    /// Return None if there is no devcontainer.json file, and treat the project as one that
    /// does not use devcontainers.
    ///
    /// From the devcontainer reference:
    /// https://containers.dev/implementors/spec/#devcontainerjson
    ///
    /// Products using it should expect to find a devcontainer.json file in one or more of the
    /// following locations (in order of precedence):
    ///
    /// * .devcontainer/devcontainer.json
    /// * .devcontainer.json
    /// * .devcontainer/<folder>/devcontainer.json (where <folder> is a sub-folder, one level deep)
    ///
    /// It is valid that these files may exist in more than one location, so consider providing a
    /// mechanism for users to select one when appropriate.
    ///
    // TODO: Allow a user to select from multiple devcontainer.json files.
    pub(crate) fn find_config(dir: &Path) -> Option<PathBuf> {
        let candidates = [
            dir.join(".devcontainer/devcontainer.json"),
            dir.join(".devcontainer.json"),
        ];

        candidates.into_iter().find(|p| p.is_file()).or_else(|| {
            // .devcontainer/<folder>/devcontainer.json
            let devcontainer_dir = dir.join(".devcontainer");
            std::fs::read_dir(&devcontainer_dir)
                .ok()
                .and_then(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .find(|e| {
                            e.file_type().is_ok_and(|ft| ft.is_dir())
                                && e.path().join("devcontainer.json").is_file()
                        })
                        .map(|e| e.path().join("devcontainer.json"))
                })
        })
    }

    /// Load the given path
    pub(crate) fn load(path: &Path) -> eyre::Result<Self> {
        // serde's flatten messes with the ability to trace what failed; so we parse the individual
        // sections separately.
        let json = std::fs::read_to_string(path)
            .wrap_err_with(|| format!("failed to read {}", path.display()))?;

        fn parse<'de, T: Deserialize<'de>>(
            json: &'de str,
            label: &str,
            path: &std::path::Path,
        ) -> eyre::Result<T> {
            let jd = &mut serde_json::Deserializer::from_str(json);
            serde_path_to_error::deserialize(jd)
                .wrap_err_with(|| format!("failed to parse {label} in {}", path.display()))
        }

        Ok(DevcontainerConfig {
            common: parse(&json, "common properties", path)?,
            kind: parse(&json, "container type properties", path)?,
        })
    }
}

#[serde_as]
#[derive(Deserialize, Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Compose {
    /// The name of the docker-compose file(s) used to start the services.
    #[serde_as(as = "OneOrMany<_>")]
    pub(crate) docker_compose_file: Vec<String>,
    /// The service you want to work on. This is considered the primary container for your dev
    /// environment which your editor will connect to.
    pub(crate) service: String,
    /// An array of services that should be started and stopped.
    #[serde(default)]
    pub(crate) run_services: Option<Vec<String>>,
    /// The path of the workspace folder inside the container. This is typically the target path of
    /// a volume mount in the docker-compose.yml.
    pub(crate) workspace_folder: PathBuf,
    /// Action to take when the user disconnects from the primary container in their editor. The
    /// default is to stop all of the compose containers.
    #[serde(default)]
    pub(crate) shutdown_action: ComposeShutdownAction,
    /// Whether to overwrite the command specified in the image. The default is false.
    #[serde(default)]
    pub(crate) override_command: bool,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct Image {
    pub(crate) image: String,

    #[serde(flatten)]
    pub(crate) non_compose: NonComposeProperties,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct Dockerfile {
    /// The location of the Dockerfile that defines the contents of the container. The path is
    /// relative to the folder containing the `devcontainer.json` file
    pub(crate) docker_file: Option<PathBuf>,
    /// The location of the context folder for building the Docker image. The path is relative to
    /// the folder containing the `devcontainer.json` file."
    pub(crate) context: Option<PathBuf>,
    pub(crate) build: Option<BuildOptions>,

    #[serde(flatten)]
    pub(crate) non_compose: NonComposeProperties,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct Common {
    /// The JSON schema of the devcontainer.json file.
    #[serde(rename = "$schema")]
    pub(crate) schema: Option<String>,
    /// A name for the dev container which can be displayed to the user.
    pub(crate) name: Option<String>,
    /// Features to add to the dev container.
    #[serde(deserialize_with = "unsupported::features::warn")]
    pub(crate) features: serde_json::Value,
    /// Array consisting of the Feature id (without the semantic version) of Features in the order
    /// the user wants them to be installed.
    #[serde(deserialize_with = "unsupported::overrideFeatureInstallOrder::warn")]
    pub(crate) override_feature_install_order: Vec<String>,
    #[serde(deserialize_with = "unsupported::secrets::warn")]
    pub(crate) secrets: serde_json::Value,
    pub(crate) forward_ports: Vec<ForwardPort>,
    pub(crate) ports_attributes: IndexMap<String, PortAttributes>,
    /// Set default properties that are applied to all ports that don't get properties from the
    /// setting `remote.portsAttributes`
    #[serde(deserialize_with = "unsupported::otherPortsAttributes::warn")]
    pub(crate) other_ports_attributes: Option<PortAttributes>,
    /// Controls whether on Linux the container's user should be updated with the local user's UID
    /// and GID. On by default when opening from a local folder.
    pub(crate) update_remote_user_uid: Option<bool>,
    /// Container environment variables.
    pub(crate) container_env: IndexMap<String, String>,
    /// The user the container will be started with. The default is the user on the Docker image.
    pub(crate) container_user: Option<String>,
    #[serde(deserialize_with = "unsupported::mounts::warn")]
    pub(crate) mounts: Vec<MountEntry>,
    /// Passes the --init flag when creating the dev container.
    pub(crate) init: Option<bool>,
    /// Passes the --privileged flag when creating the dev container.
    pub(crate) privileged: Option<bool>,
    /// Passes docker capabilities to include when creating the dev container.
    pub(crate) cap_add: Vec<String>,
    /// Passes docker security options to include when creating the dev container.
    pub(crate) security_opt: Vec<String>,
    /// Remote environment variables to set for processes spawned in the
    /// container including lifecycle scripts and any remote editor/IDE server
    /// process.
    pub(crate) remote_env: IndexMap<String, Option<String>>,
    /// The username to use for spawning processes in the container including
    /// lifecycle scripts and any remote editor/IDE server process. The default
    /// is the same user as the container.
    pub(crate) remote_user: Option<String>,

    /// A command to run locally (i.e Your host machine, cloud VM) before anything else. This
    /// command is run before "onCreateCommand".
    pub(crate) initialize_command: Option<LifecycleCommand>,
    /// A command to run when creating the container. This command is run after "initializeCommand"
    /// and before "updateContentCommand".
    pub(crate) on_create_command: Option<LifecycleCommand>,
    /// A command to run when creating the container and rerun when the workspace content was
    /// updated while creating the container. This command is run after "onCreateCommand" and before
    /// "postCreateCommand".
    pub(crate) update_content_command: Option<LifecycleCommand>,
    /// A command to run after creating the container. This command is run after
    /// "updateContentCommand" and before "postStartCommand".
    pub(crate) post_create_command: Option<LifecycleCommand>,
    /// A command to run after starting the container. This command is run after "postCreateCommand"
    /// and before "postAttachCommand".
    pub(crate) post_start_command: Option<LifecycleCommand>,
    /// A command to run when attaching to the container. This command is run after
    /// "postStartCommand".
    pub(crate) post_attach_command: Option<LifecycleCommand>,
    /// The user command to wait for before continuing execution in the background while the UI is
    /// starting up.
    pub(crate) wait_for: WaitFor,
    /// User environment probe to run.
    pub(crate) user_env_probe: UserEnvProbe,

    /// Host hardware requirements.
    pub(crate) host_requirements: Option<HostRequirements>,
    /// Tool-specific configuration. Each tool should use a JSON object subproperty with a unique
    /// name to group its customizations.
    pub(crate) customizations: Customizations,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub(crate) struct Customizations {
    #[serde(default)]
    pub(crate) devconcurrent: DcOptions,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(untagged)]
pub(crate) enum Port {
    Number(u16),
    String(String),
}

#[serde_as]
#[serde_inline_default]
#[derive(Deserialize, Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct NonComposeProperties {
    /// Application ports that are exposed by the container. This can be a single port or an array
    /// of ports. Each port can be a number or a string. A number is mapped to the same port on the
    /// host. A string is passed to Docker unchanged and can be used to map ports differently, e.g.
    /// "8000:8010".
    #[serde_as(as = "OneOrMany<_>")]
    pub(crate) app_port: Vec<Port>,
    /// The arguments required when starting in the container.
    pub(crate) run_args: Vec<String>,
    /// Action to take when the user disconnects from the container in their editor. The default is
    /// to stop the container.
    pub(crate) shutdown_action: NonComposeShutdownAction,
    /// Whether to overwrite the command specified in the image. The default is true.
    #[serde_inline_default(true)]
    pub(crate) override_command: bool,
    /// The path of the workspace folder inside the container.
    pub(crate) workspace_folder: Option<PathBuf>,
    /// The --mount parameter for docker run. The default is to mount the project folder at
    /// /workspaces/$project.
    pub(crate) workspace_mount: Option<PathBuf>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(untagged)]
pub(crate) enum MountEntry {
    String(String),
    Object(Mount),
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Mount {
    #[serde(rename = "type")]
    pub(crate) ty: MountType,
    #[serde(default)]
    pub(crate) source: Option<String>,
    pub(crate) target: String,
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MountType {
    Bind,
    Volume,
}

#[serde_as]
#[derive(Deserialize, Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct BuildOptions {
    pub(crate) target: Option<String>,
    pub(crate) args: IndexMap<String, String>,
    #[serde_as(as = "OneOrMany<_>")]
    pub(crate) cache_from: Vec<String>,
    pub(crate) options: Vec<String>,
}

#[serde_inline_default]
#[derive(Deserialize, Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct HostRequirements {
    /// Number of required CPUs. Minimum 1.
    #[serde_inline_default(1)]
    pub(crate) cpus: u64,
    /// Amount of required RAM in bytes. Supports units tb, gb, mb and kb.
    pub(crate) memory: Option<String>,
    /// Amount of required RAM in bytes. Supports units tb, gb, mb and kb.
    pub(crate) storage: Option<String>,
    pub(crate) gpu: GpuRequirement,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(untagged)]
pub(crate) enum GpuRequirement {
    Bool(bool),
    String(GpuOptional),
    Object {
        /// Number of cores. Minimum 1.
        cores: Option<u64>,
        /// Amount of required RAM in bytes. Supports units tb, gb, mb and kb.
        memory: Option<String>,
    },
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum GpuOptional {
    Optional,
}

impl Default for GpuRequirement {
    fn default() -> Self {
        Self::Bool(false)
    }
}

#[serde_inline_default]
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PortAttributes {
    #[serde(default)]
    pub(crate) on_auto_forward: OnAutoForward,
    #[serde(default)]
    pub(crate) elevate_if_needed: bool,
    #[serde_inline_default(String::from("Application"))]
    pub(crate) label: String,
    #[serde(default)]
    pub(crate) protocol: Protocol,
    #[serde(default)]
    pub(crate) require_local_port: bool,
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Protocol {
    #[default]
    Http,
    Https,
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) enum OnAutoForward {
    #[default]
    Notify,
    OpenBrowser,
    OpenBrowserOnce,
    OpenPreview,
    Silent,
    Ignore,
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) enum UserEnvProbe {
    None,
    LoginShell,
    #[default]
    LoginInteractiveShell,
    InteractiveShell,
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) enum WaitFor {
    InitializeCommand,
    OnCreateCommand,
    #[default]
    UpdateContentCommand,
    PostCreateCommand,
    PostStartCommand,
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ComposeShutdownAction {
    None,
    #[default]
    StopCompose,
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) enum NonComposeShutdownAction {
    None,
    #[default]
    StopContainer,
}
