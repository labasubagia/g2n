use clap::Parser;
use tracing_subscriber::EnvFilter;

use g2n::cli::app::{Cli, run};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let cli = Cli::parse();
    if let Err(error) = run(cli).await {
        eprintln!("Error: {error:#}");
        std::process::exit(1);
    }
}
