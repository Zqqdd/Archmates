//! `archmates`: the command line and the background service (`archmates daemon`).
//! See `src/cli/` for the commands and the `archmates` library for everything else.

mod cli;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    cli::run().await
}
