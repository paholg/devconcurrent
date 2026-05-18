//! Integration tests for `Docker::inspect_image` and `Docker::pull_image`.
//!
//! Gated behind the `docker-tests` feature; needs a live daemon.

#![cfg(feature = "docker-tests")]

mod helpers;

use docker::{Docker, Error};

const IMAGE: &str = "alpine:3.20";

#[tokio::test]
async fn inspect_returns_not_found_for_unknown_image() {
    let client = Docker::connect().await.expect("connect");
    let err = client
        .inspect_image("docker-crate-test/does-not-exist:zzz")
        .await
        .expect_err("missing image should error");
    assert!(
        matches!(err, Error::NotFound),
        "expected NotFound, got {err:?}"
    );
}

#[tokio::test]
async fn pull_then_inspect_succeeds() {
    let client = Docker::connect().await.expect("connect");
    client.pull_image(IMAGE).await.expect("pull");
    let details = client.inspect_image(IMAGE).await.expect("inspect");
    assert!(
        details.repo_tags.iter().any(|t| t.contains("alpine")),
        "repo_tags should include the alpine tag, got {:?}",
        details.repo_tags
    );
}

#[tokio::test]
async fn pull_unknown_image_returns_error() {
    let client = Docker::connect().await.expect("connect");
    let err = client
        .pull_image("docker-crate-test/no-such-image:zzz")
        .await
        .expect_err("pull of non-existent image should fail");
    // Either:
    // - the daemon returns 404 directly (mapped to NotFound), or
    // - it returns 200 and emits an error event mid-stream (mapped to Api).
    // Both are legitimate outcomes; the test only cares that the pull failed.
    assert!(
        matches!(err, Error::Api { .. } | Error::NotFound),
        "expected Api or NotFound, got {err:?}",
    );
}
