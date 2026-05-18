//! Integration tests for `Docker::stats`.
//!
//! Gated behind the `docker-tests` feature; needs a live daemon.

#![cfg(feature = "docker-tests")]

mod helpers;

use docker::Docker;

use helpers::TestContainer;

const IMAGE: &str = "alpine:3.20";

#[tokio::test]
async fn stats_returns_memory_usage_for_running_container() {
    let container = TestContainer::start(IMAGE, &["sleep", "60"]);
    let client = Docker::connect().await.expect("connect");

    let stats = client.stats(container.id()).await.expect("stats");

    // memory_stats.usage may be `None` on cgroup v1 without memory accounting,
    // but on any modern Linux test runner we expect it populated.
    let usage = stats.memory_stats.usage.expect("usage should be reported");
    assert!(usage > 0, "running container should use some memory");
}
