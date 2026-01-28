#[cfg(feature = "gateway")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or(
        "usage: ditto-gateway <config.json> [--listen HOST:PORT] [--admin-token TOKEN] [--backend name=url]",
    )?;

    let mut listen = "127.0.0.1:8080".to_string();
    let mut admin_token: Option<String> = None;
    let mut backend_specs: Vec<String> = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--listen" | "--addr" => {
                listen = args.next().ok_or("missing value for --listen/--addr")?;
            }
            "--admin-token" => {
                admin_token = Some(args.next().ok_or("missing value for --admin-token")?);
            }
            "--backend" => {
                backend_specs.push(args.next().ok_or("missing value for --backend")?);
            }
            other => return Err(format!("unknown arg: {other}").into()),
        }
    }

    let raw = std::fs::read_to_string(&path)?;
    let config: ditto_llm::gateway::GatewayConfig = serde_json::from_str(&raw)?;
    let mut gateway = ditto_llm::gateway::Gateway::new(config);

    for spec in backend_specs {
        let (name, url) = spec
            .split_once('=')
            .ok_or("backend spec must be name=url")?;
        let backend = ditto_llm::gateway::HttpBackend::new(url)?;
        gateway.register_backend(name.to_string(), backend);
    }

    let mut state = ditto_llm::gateway::GatewayHttpState::new(gateway);
    if let Some(token) = admin_token {
        state = state.with_admin_token(token);
    }

    let app = ditto_llm::gateway::http::router(state);
    let listener = tokio::net::TcpListener::bind(&listen).await?;
    println!("ditto-gateway listening on {listen}");
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(not(feature = "gateway"))]
fn main() {
    eprintln!("gateway feature disabled; rebuild with --features gateway");
}
