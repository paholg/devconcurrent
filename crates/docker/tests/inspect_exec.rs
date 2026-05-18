//! Integration tests for `Docker::inspect_exec`.
//!
//! Gated behind the `docker-tests` feature; needs a live daemon.

#![cfg(feature = "docker-tests")]

mod helpers;

use std::process::Command;

use docker::{Docker, Error};

use helpers::TestContainer;

const IMAGE: &str = "alpine:3.20";

#[tokio::test]
async fn inspect_exec_returns_running_for_live_exec() {
    let container = TestContainer::start(IMAGE, &["sleep", "60"]);

    // Start a background exec so the container has an ExecID to inspect.
    let status = Command::new("docker")
        .args(["exec", "-d", container.id(), "sleep", "30"])
        .status()
        .expect("docker exec");
    assert!(status.success());

    let client = Docker::connect().await.expect("connect");
    let details = client
        .inspect_container(container.id())
        .await
        .expect("inspect container");
    let exec_id = details
        .exec_ids
        .first()
        .expect("exec_ids should be populated after `docker exec -d`")
        .clone();

    let exec = client.inspect_exec(&exec_id).await.expect("inspect exec");
    assert_eq!(exec.id, exec_id);
    assert!(exec.running, "exec should still be running");
    assert!(exec.exit_code.is_none(), "no exit code while still running");
}

#[tokio::test]
async fn inspect_exec_returns_not_found_for_missing() {
    let client = Docker::connect().await.expect("connect");
    let err = client
        .inspect_exec("docker-crate-test-no-such-exec")
        .await
        .expect_err("missing exec should error");
    assert!(
        matches!(err, Error::NotFound),
        "expected NotFound, got {err:?}"
    );
}
