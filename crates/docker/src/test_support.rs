//! Test helpers for integration tests in `tests/`.

use rand::distr::{Alphanumeric, SampleString};

use crate::Docker;

/// Applied to every test-managed container.
pub const TEST_LABEL: &str = "devconcurrent-docker-crate-test=true";

/// RAII guard around a running container.
///
/// Construction calls the Docker API to pull, create, and start. `Drop`
/// removes the container via the API and ignores errors — the goal is
/// best-effort cleanup, including on panic.
///
/// Drop blocks the current thread by trampolining into the surrounding tokio
/// runtime, so tests using this guard must run on a multi-threaded runtime
/// (`#[tokio::test(flavor = "multi_thread")]`).
pub struct TestContainer {
    client: Docker,
    name: String,
}

impl TestContainer {
    /// Pull `image` if needed, then create and start a container running `cmd`,
    /// with a high-entropy random name and the test label applied.
    pub async fn start(client: &Docker, image: &str, cmd: &[&str]) -> Self {
        let name = unique_name();
        client.ensure_image(image).await.expect("ensure_image");
        let (test_key, test_value) = TEST_LABEL.split_once('=').expect("TEST_LABEL is key=value");
        let cmd: Vec<String> = cmd.iter().map(|s| (*s).to_string()).collect();
        let id = client
            .create_container(&name)
            .image(image)
            .cmd(cmd)
            .with_label(test_key, test_value)
            .call()
            .await
            .unwrap_or_else(|e| panic!("create_container({name}, {image}) failed: {e}"));
        client
            .start_container(&id)
            .await
            .unwrap_or_else(|e| panic!("start_container({name}) failed: {e}"));
        Self {
            client: client.clone(),
            name,
        }
    }

    pub fn id(&self) -> &str {
        &self.name
    }
}

impl Drop for TestContainer {
    fn drop(&mut self) {
        force_remove_container(&self.client, &self.name);
    }
}

pub fn unique_name() -> String {
    let suffix = Alphanumeric.sample_string(&mut rand::rng(), 24);
    format!("devconcurrent-docker-crate-test-{suffix}")
}

/// RAII cleanup of a container the test created itself (e.g. via the API
/// under test). On drop, removes the container via the API and ignores errors.
pub struct ContainerCleanup {
    pub client: Docker,
    pub name: String,
}

impl Drop for ContainerCleanup {
    fn drop(&mut self) {
        force_remove_container(&self.client, &self.name);
    }
}

/// RAII cleanup of a volume the test created. Best-effort, ignores errors.
pub struct VolumeCleanup {
    pub client: Docker,
    pub name: String,
}

impl Drop for VolumeCleanup {
    fn drop(&mut self) {
        let client = self.client.clone();
        let name = self.name.clone();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let _ = client.remove_volume(&name).force(true).call().await;
            });
        });
    }
}

fn force_remove_container(client: &Docker, name: &str) {
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async {
            let _ = client.remove_container(name).force(true).call().await;
        });
    });
}
