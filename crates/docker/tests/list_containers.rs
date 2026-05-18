//! Integration tests for `Docker::list_containers`.
//!
//! Gated behind the `docker-tests` feature; needs a live daemon.

#![cfg(feature = "docker-tests")]

use docker::{ContainerStatus, Docker};

use docker::test_support::{TEST_LABEL, TestContainer};

const IMAGE: &str = "alpine:3.20";

#[tokio::test(flavor = "multi_thread")]
async fn lists_only_running_by_default() {
    let client = Docker::connect().await.expect("connect");
    let container = TestContainer::start(&client, IMAGE, &["sleep", "60"]).await;
    let (key, value) = TEST_LABEL.split_once('=').expect("TEST_LABEL is key=value");

    let summaries = client
        .list_containers()
        .with_label(key, value)
        .call()
        .await
        .expect("list");

    assert!(
        summaries.iter().any(|s| s.id == container.id()
            || s.names
                .iter()
                .any(|n| n.trim_start_matches('/') == container.id())),
        "newly-started container should be in the list",
    );
    assert!(
        summaries
            .iter()
            .all(|s| s.state == ContainerStatus::Running),
        "default list_containers should return only running entries",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn filter_label_narrows_results() {
    let client = Docker::connect().await.expect("connect");
    let _container = TestContainer::start(&client, IMAGE, &["sleep", "60"]).await;

    let summaries = client
        .list_containers()
        .with_label("no-such-key-zzzzz", "value")
        .call()
        .await
        .expect("list");

    assert!(
        summaries.is_empty(),
        "filtering on a label nothing has should return zero results, got {}",
        summaries.len(),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn all_includes_stopped() {
    let client = Docker::connect().await.expect("connect");
    let container = TestContainer::start(&client, IMAGE, &["true"]).await;

    // Wait briefly for the container to exit on its own.
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let details = client
            .inspect_container(container.id())
            .await
            .expect("inspect");
        if details.state.status != ContainerStatus::Running {
            break;
        }
    }

    let (key, value) = TEST_LABEL.split_once('=').expect("TEST_LABEL is key=value");
    let summaries = client
        .list_containers()
        .all(true)
        .with_label(key, value)
        .call()
        .await
        .expect("list");

    assert!(
        summaries.iter().any(|s| s
            .names
            .iter()
            .any(|n| n.trim_start_matches('/') == container.id())),
        "with all=true, exited container should be in the list",
    );
}
