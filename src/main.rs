//! sd-agent CLI 薄壳：解析参数 → 调 lib，零业务逻辑（决策 1）。

#[tokio::main]
async fn main() {
    let code = sd_agent::cli::run().await;
    std::process::exit(code);
}
