//! Integration tests for the volume API.
//!
//! Gated behind the `docker-tests` feature; needs a live daemon.

#![cfg(feature = "docker-tests")]

mod helpers;

use std::process::Command;

use docker::{Docker, Error};
use rand::RngExt;
use rand::distr::Alphanumeric;

use helpers::TEST_LABEL;

struct TestVolume {
    name: String,
}

impl TestVolume {
    fn new() -> Self {
        let suffix: String = rand::rng()
            .sample_iter(Alphanumeric)
            .take(24)
            .map(char::from)
            .collect();
        Self {
            name: format!("devconcurrent-docker-crate-test-vol-{suffix}"),
        }
    }
}

impl Drop for TestVolume {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["volume", "rm", "-f", &self.name])
            .output();
    }
}

#[tokio::test]
async fn create_then_list_then_remove() {
    let v = TestVolume::new();
    let (key, value) = TEST_LABEL.split_once('=').expect("TEST_LABEL is key=value");
    let client = Docker::connect().await.expect("connect");

    let created = client
        .create_volume(&v.name)
        .with_label(key, value)
        .call()
        .await
        .expect("create");
    assert_eq!(created.name, v.name);
    assert_eq!(created.labels.get(key).map(String::as_str), Some(value));

    let listed = client
        .list_volumes()
        .with_label(key, value)
        .call()
        .await
        .expect("list");
    assert!(
        listed.iter().any(|x| x.name == v.name),
        "created volume should be in the labelled list"
    );

    client.remove_volume(&v.name).call().await.expect("remove");
}

#[tokio::test]
async fn remove_missing_volume_returns_not_found() {
    let client = Docker::connect().await.expect("connect");
    let err = client
        .remove_volume("docker-crate-test-no-such-vol")
        .call()
        .await
        .expect_err("missing volume should error");
    assert!(
        matches!(err, Error::NotFound),
        "expected NotFound, got {err:?}",
    );
}
