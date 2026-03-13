use opentelemetry::global;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::SpanExporter;
use opentelemetry_otlp::WithExportConfig as _;
use opentelemetry_sdk::Resource;
use tracing_subscriber::Layer as _;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

#[derive(Debug)]
pub struct OtelGuard {
    provider: opentelemetry_sdk::trace::SdkTracerProvider,
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        let _ = self.provider.shutdown();
    }
}

pub fn init_tracing(
    service_name: &str,
    endpoint: Option<&str>,
    json_logs: bool,
) -> Result<OtelGuard, Box<dyn std::error::Error>> {
    let mut exporter = SpanExporter::builder().with_http();
    if let Some(endpoint) = endpoint {
        exporter = exporter.with_endpoint(endpoint.to_string());
    }
    let exporter = exporter.build()?;

    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(
            Resource::builder_empty()
                .with_service_name(service_name.to_string())
                .build(),
        )
        .build();
    global::set_tracer_provider(provider.clone());

    let tracer = provider.tracer(service_name.to_string());
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    let fmt_layer = if json_logs {
        tracing_subscriber::fmt::layer()
            .json()
            .with_target(false)
            .boxed()
    } else {
        tracing_subscriber::fmt::layer().with_target(false).boxed()
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .with(otel_layer)
        .try_init()?;

    Ok(OtelGuard { provider })
}
