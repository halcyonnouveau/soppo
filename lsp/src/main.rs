use clap::Parser;
use sopls::Cli;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    sopls::run_server(cli).await;
}
