#![forbid(unsafe_code)]

use clap::{CommandFactory, Parser};
use clap_complete::CompleteEnv;
use color_eyre::config::HookBuilder;
use dc::{self, cli::Cli};

#[tokio::main(flavor = "current_thread")]
async fn main() -> eyre::Result<()> {
    CompleteEnv::with_factory(Cli::command).complete();

    HookBuilder::default()
        .display_env_section(false)
        .install()?;
    dc::subscriber::init_subscriber();

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            // Ensure help/version/error output goes to stderr, not stdout
            eprintln!("{}", e.render().ansi());
            std::process::exit(e.exit_code());
        }
    };
    cli.run().await
}
