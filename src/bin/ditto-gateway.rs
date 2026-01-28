#[cfg(feature = "gateway")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: ditto-gateway <config.json>")?;
    let raw = std::fs::read_to_string(&path)?;
    let config: ditto_llm::gateway::GatewayConfig = serde_json::from_str(&raw)?;

    println!(
        "gateway config loaded: virtual_keys={}, default_backend={}",
        config.virtual_keys.len(),
        config.router.default_backend
    );
    println!("gateway placeholder ready; wire up service here.");
    Ok(())
}

#[cfg(not(feature = "gateway"))]
fn main() {
    eprintln!("gateway feature disabled; rebuild with --features gateway");
}
