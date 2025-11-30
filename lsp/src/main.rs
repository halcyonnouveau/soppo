#[tokio::main]
async fn main() {
    soppo_lsp::run_server().await;
}
