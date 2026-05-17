//! Integration tests for `Docker::inspect_container`.
//!
//! Gated behind the `docker-tests` feature; build with
//! `cargo nextest run -p docker --features docker-tests` (or via the
//! workspace-level `cargo nextest run --features docker/docker-tests`).
//! These talk to a real docker (or podman-with-docker-compat) daemon.
//!
//! Each test uses [`helpers::TestContainer`], an RAII guard that removes its
//! container on Drop (including during panic). CI also sweeps all containers
//! labelled `devconcurrent-docker-crate-test=true` before/after each run as
//! a backstop.

#![cfg(feature = "docker-tests")]

mod helpers;

use docker::{ContainerStatus, Docker, Error};

use helpers::{TEST_LABEL, TestContainer};

const IMAGE: &str = "alpine:3.20";

#[tokio::test]
async fn inspect_returns_running_container_details() {
    let container = TestContainer::start(IMAGE, &["sleep", "60"]);
    let client = Docker::connect().await.expect("connect");

    let details = client.inspect_container(container.id()).await.unwrap();

    assert_eq!(details.state.status, ContainerStatus::Running);
    assert!(details.state.running);
    assert_eq!(details.state.exit_code, 0);
    assert!(!details.id.is_empty(), "container id should be populated");
}

#[tokio::test]
async fn inspect_returns_not_found_for_missing_container() {
    let client = Docker::connect().await.expect("connect");

    let err = client
        .inspect_container("docker-crate-test-does-not-exist")
        .await
        .expect_err("missing container should error");

    assert!(
        matches!(err, Error::NotFound),
        "expected Error::NotFound, got {err:?}",
    );
}

#[tokio::test]
async fn inspect_surfaces_container_labels() {
    let container = TestContainer::start(IMAGE, &["sleep", "60"]);
    let client = Docker::connect().await.expect("connect");

    let details = client
        .inspect_container(container.id())
        .await
        .expect("inspect");

    // Our test guard always sets `TEST_LABEL`; if we can read it back through
    // inspect, label propagation is working end-to-end.
    let (key, value) = TEST_LABEL.split_once('=').expect("TEST_LABEL is key=value");
    assert_eq!(
        details.config.labels.get(key).map(String::as_str),
        Some(value),
    );
}
