use std::process::Command;

use rand::distr::{Alphanumeric, SampleString};

/// Applied to every test-managed container. CI can sweep stragglers with
/// `docker rm -f $(docker ps -aq --filter "label=<TEST_LABEL>")`.
pub const TEST_LABEL: &str = "devconcurrent-docker-crate-test=true";

/// RAII guard around a running container.
///
/// Construction shells out to `docker run -d` and panics if creation fails.
/// `Drop` shells out to `docker rm -f` and intentionally ignores errors — the
/// goal is best-effort cleanup, including on panic.
///
/// Process termination (SIGKILL, OOM, etc.) bypasses `Drop`, so CI should
/// also sweep [`TEST_LABEL`] before and after each job as a backstop.
pub struct TestContainer {
    name: String,
}

impl TestContainer {
    /// Start a container from `image` running `cmd`, with a high-entropy
    /// random name and the test label applied.
    pub fn start(image: &str, cmd: &[&str]) -> Self {
        let name = unique_name();
        let mut args: Vec<&str> = vec!["run", "-d", "--name", &name, "--label", TEST_LABEL, image];
        args.extend_from_slice(cmd);
        let output = Command::new("docker")
            .args(&args)
            .output()
            .expect("failed to invoke `docker run`");
        assert!(
            output.status.success(),
            "`docker run` failed for image {image:?}: {}",
            String::from_utf8_lossy(&output.stderr).trim(),
        );
        Self { name }
    }

    pub fn id(&self) -> &str {
        &self.name
    }
}

impl Drop for TestContainer {
    fn drop(&mut self) {
        // Best-effort: a failing cleanup shouldn't mask the original panic.
        let _ = Command::new("docker")
            .args(["rm", "-f", &self.name])
            .output();
    }
}

pub fn unique_name() -> String {
    let suffix = Alphanumeric.sample_string(&mut rand::rng(), 24);
    format!("devconcurrent-docker-crate-test-{suffix}")
}

/// RAII cleanup of a container the test created itself (e.g. via the API
/// under test). On drop, shells out to `docker rm -f` and ignores errors.
pub struct ContainerCleanup {
    pub name: String,
}

impl Drop for ContainerCleanup {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "-f", &self.name])
            .output();
    }
}
