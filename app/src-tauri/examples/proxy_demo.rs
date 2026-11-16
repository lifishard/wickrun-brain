//! Run only the translator, for manual checks with a real Claude Code:
//!   WB_UPSTREAM=http://127.0.0.1:9000/v1 WB_UPSTREAM_KEY=k WB_MODEL=m cargo run --example proxy_demo
//! Prints the port and token to use as ANTHROPIC_BASE_URL / ANTHROPIC_AUTH_TOKEN.
#[tokio::main]
async fn main() {
    let token = wickrun_brain_lib::proxy::new_token();
    let p = wickrun_brain_lib::proxy::start(0, token.clone()).await.expect("start");
    *p.inner.route.write().await = Some(wickrun_brain_lib::proxy::Route {
        upstream_base: std::env::var("WB_UPSTREAM").expect("WB_UPSTREAM"),
        upstream_key: std::env::var("WB_UPSTREAM_KEY").expect("WB_UPSTREAM_KEY"),
        model: std::env::var("WB_MODEL").expect("WB_MODEL"),
        reasoning_effort: std::env::var("WB_REASONING_EFFORT").ok(),
    });
    println!("ANTHROPIC_BASE_URL=http://127.0.0.1:{}", p.port);
    println!("ANTHROPIC_AUTH_TOKEN={}", token);
    tokio::signal::ctrl_c().await.ok();
}
