//! Docker events listener: watches for workspace devcontainer containers
//! starting/dying and drives sidecar lifecycle through the registry.

use docker::{Docker, EventActor};
use futures_util::StreamExt;
use shared::{COMPOSE_PROJECT_LABEL, COMPOSE_SERVICE_LABEL, PROJECT_LABEL, WORKSPACE_LABEL};

use crate::registry::{Registry, RunningWorkspace};
use crate::sidecar;

/// Run the event loop. Reconnects on connection drops with a brief backoff.
pub async fn run(docker: Docker, registry: Registry) {
    loop {
        let stream = match docker
            .events()
            .with_type("container")
            .with_label_key(PROJECT_LABEL)
            .with_event("start")
            .with_event("die")
            .with_event("destroy")
            .call()
            .await
        {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("failed to open docker events: {e}; retrying in 2s");
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
        };
        tokio::pin!(stream);
        tracing::info!("subscribed to docker events");

        while let Some(item) = stream.next().await {
            match item {
                Ok(ev) => handle_event(&docker, &registry, ev).await,
                Err(e) => {
                    tracing::warn!("docker events stream error: {e}");
                    break;
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

async fn handle_event(docker: &Docker, registry: &Registry, ev: docker::EventMessage) {
    let Some(action) = ev.action.as_deref() else {
        return;
    };
    match action {
        "start" => on_start(docker, registry, ev.actor).await,
        "die" | "destroy" => on_die(docker, registry, ev.actor).await,
        _ => {}
    }
}

async fn on_start(docker: &Docker, registry: &Registry, actor: EventActor) {
    let attrs = &actor.attributes;
    let Some(project) = attrs.get(PROJECT_LABEL).cloned() else {
        return;
    };
    let compose_service = attrs
        .get(COMPOSE_SERVICE_LABEL)
        .cloned()
        .unwrap_or_default();

    let Some(cfg) = registry.config_for(&project).await else {
        tracing::debug!(
            container = %actor.id,
            project,
            "container started but no config registered; ignoring"
        );
        return;
    };

    if compose_service != cfg.devcontainer_service {
        // Sibling compose service — the existing sidecar (if any) already
        // proxies it via compose DNS from inside the netns. Nothing to do.
        return;
    }

    let Some(workspace) = derive_workspace(attrs) else {
        tracing::warn!(
            container = %actor.id,
            project,
            "container has project label but no workspace identifier; skipping"
        );
        return;
    };

    tracing::info!(
        container = %actor.id,
        project,
        workspace,
        "workspace devcontainer started; creating sidecar"
    );

    let sidecar_id = match sidecar::create_sidecar(docker, &cfg, &workspace, &actor.id).await {
        Ok(id) => Some(id),
        Err(e) => {
            tracing::error!(
                project,
                workspace,
                target_cid = %actor.id,
                "create sidecar failed: {e:?}"
            );
            None
        }
    };

    let ws = RunningWorkspace {
        project,
        workspace,
        target_cid: actor.id.clone(),
        sidecar_id,
    };
    registry.track_workspace(ws).await;
}

async fn on_die(docker: &Docker, registry: &Registry, actor: EventActor) {
    let Some(ws) = registry.untrack_workspace(&actor.id).await else {
        return;
    };
    if let Some(sidecar_id) = ws.sidecar_id {
        sidecar::remove_sidecar(docker, &sidecar_id).await;
    }
}

/// Pick a workspace identifier from the available labels, preferring the
/// explicit `WORKSPACE_LABEL` if set and falling back to the compose project
/// label (with the `_devcontainer` suffix the CLI appends stripped, if
/// present). The identifier is used as a sidecar key and as the `workspace`
/// component of rendered hostnames.
pub(crate) fn derive_workspace(attrs: &indexmap::IndexMap<String, String>) -> Option<String> {
    if let Some(ws) = attrs.get(WORKSPACE_LABEL).filter(|s| !s.is_empty()) {
        return Some(ws.clone());
    }
    let compose = attrs.get(COMPOSE_PROJECT_LABEL)?;
    if compose.is_empty() {
        return None;
    }
    Some(
        compose
            .strip_suffix("_devcontainer")
            .unwrap_or(compose)
            .to_string(),
    )
}
