//! Integration tests for `Docker::stats`.
//!
//! Gated behind the `docker-tests` feature; needs a live daemon.

#![cfg(feature = "docker-tests")]

use docker::Docker;

use docker::test_support::TestContainer;

const IMAGE: &str = "alpine:3.20";

#[tokio::test(flavor = "multi_thread")]
async fn stats_returns_memory_usage_for_running_container() {
    let client = Docker::connect().await.expect("connect");
    let container = TestContainer::start(&client, IMAGE, &["sleep", "60"]).await;

    let stats = client.stats(container.id()).await.expect("stats");

    // memory_stats.usage may be `None` on cgroup v1 without memory accounting,
    // but on any modern Linux test runner we expect it populated.
    let usage = stats.memory_stats.usage.expect("usage should be reported");
    assert!(usage > 0, "running container should use some memory");
}
