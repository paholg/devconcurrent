#![forbid(unsafe_code)]

#[tokio::main(flavor = "current_thread")]
async fn main() -> eyre::Result<()> {
    devconcurrent::cli_main().await
}
