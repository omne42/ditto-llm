#[cfg(feature = "gateway-costing")]
fn estimate_charge_cost_usd_micros(
    state: &GatewayHttpState,
    gateway: &super::Gateway,
    request_model: Option<&str>,
    input_tokens_estimate: u32,
    max_output_tokens: u32,
    service_tier: Option<&str>,
    backends: &[String],
) -> Option<u64> {
    let request_model = request_model?;
    let pricing = state.pricing.as_ref()?;

    let mut cost = pricing.estimate_cost_usd_micros_for_service_tier(
        request_model,
        input_tokens_estimate,
        max_output_tokens,
        service_tier,
    );

    for backend_name in backends {
        if !state.proxy_backends.contains_key(backend_name) {
            continue;
        }

        let mapped_model = gateway
            .config
            .backends
            .iter()
            .find(|backend| backend.name == backend_name.as_str())
            .and_then(|backend| backend.model_map.get(request_model))
            .map(|model| model.as_str());

        if let Some(mapped_model) = mapped_model {
            cost = max_option_u64(
                cost,
                pricing.estimate_cost_usd_micros_for_service_tier(
                    mapped_model,
                    input_tokens_estimate,
                    max_output_tokens,
                    service_tier,
                ),
            );
        }
    }

    cost
}
