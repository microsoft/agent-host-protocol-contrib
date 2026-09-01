#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    advanced_workspace_host::cli::run().await
}
