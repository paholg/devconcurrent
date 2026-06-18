#![forbid(unsafe_code)]

#[tokio::main(flavor = "current_thread")]
async fn main() -> eyre::Result<()> {
    // Keep writing to a closed pipe from panicking.
    sigpipe::reset();

    devconcurrent::cli_main().await
}
