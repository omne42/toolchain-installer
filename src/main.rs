mod cli;

#[tokio::main]
async fn main() {
    if let Err(err) = cli::run().await {
        eprintln!("{err}");
        std::process::exit(err.exit_code().as_i32());
    }
}
