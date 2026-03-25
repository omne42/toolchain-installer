#[path = "bootstrap/cli.rs"]
mod bootstrap_cli;

#[tokio::main]
async fn main() {
    if let Err(err) = bootstrap_cli::run().await {
        eprintln!("{err}");
        std::process::exit(err.exit_code().as_i32());
    }
}
