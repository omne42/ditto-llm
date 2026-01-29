#![cfg(feature = "gateway")]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;

use ditto_llm::gateway::{
    Backend, BudgetConfig, CacheConfig, Clock, Gateway, GatewayConfig, GatewayError,
    GatewayRequest, GatewayResponse, GuardrailsConfig, LimitsConfig, PassthroughConfig, RouteRule,
    RouterConfig, VirtualKeyConfig,
};

struct FixedClock {
    now: u64,
}

impl Clock for FixedClock {
    fn now_epoch_seconds(&self) -> u64 {
        self.now
    }
}

struct StaticBackend {
    content: String,
    calls: Arc<AtomicUsize>,
}

impl StaticBackend {
    fn new(content: impl Into<String>) -> (Self, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Self {
                content: content.into(),
                calls: calls.clone(),
            },
            calls,
        )
    }
}

#[async_trait]
impl Backend for StaticBackend {
    async fn call(&self, _request: &GatewayRequest) -> Result<GatewayResponse, GatewayError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(GatewayResponse {
            content: self.content.clone(),
            output_tokens: 1,
            backend: String::new(),
            cached: false,
        })
    }
}

fn base_request() -> GatewayRequest {
    GatewayRequest {
        virtual_key: "vk-1".to_string(),
        model: "gpt-4o-mini".to_string(),
        prompt: "hello".to_string(),
        input_tokens: 4,
        max_output_tokens: 6,
        passthrough: false,
    }
}

fn base_key() -> VirtualKeyConfig {
    VirtualKeyConfig {
        id: "key-1".to_string(),
        token: "vk-1".to_string(),
        enabled: true,
        limits: LimitsConfig::default(),
        budget: BudgetConfig::default(),
        cache: CacheConfig::default(),
        guardrails: GuardrailsConfig::default(),
        passthrough: PassthroughConfig::default(),
        route: None,
    }
}

fn base_config(key: VirtualKeyConfig) -> GatewayConfig {
    GatewayConfig {
        backends: Vec::new(),
        virtual_keys: vec![key],
        router: RouterConfig {
            default_backend: "primary".to_string(),
            default_backends: Vec::new(),
            rules: Vec::new(),
        },
    }
}

#[tokio::test]
async fn virtual_key_validation_rejects_unknown() {
    let config = base_config(base_key());
    let clock = Box::new(FixedClock { now: 0 });
    let mut gateway = Gateway::with_clock(config, clock);

    let mut request = base_request();
    request.virtual_key = "invalid".to_string();
    let err = gateway.handle(request).await.unwrap_err();
    assert!(matches!(err, GatewayError::Unauthorized));
}

#[tokio::test]
async fn rate_limit_rpm_blocks_second_request() {
    let mut key = base_key();
    key.limits = LimitsConfig {
        rpm: Some(1),
        tpm: None,
    };
    let config = base_config(key);
    let clock = Box::new(FixedClock { now: 120 });
    let mut gateway = Gateway::with_clock(config, clock);

    let (backend, _calls) = StaticBackend::new("ok");
    gateway.register_backend("primary", backend);

    let request = base_request();
    gateway.handle(request.clone()).await.unwrap();
    let err = gateway.handle(request).await.unwrap_err();
    assert!(matches!(err, GatewayError::RateLimited { .. }));
}

#[tokio::test]
async fn rate_limit_tpm_blocks_overuse() {
    let mut key = base_key();
    key.limits = LimitsConfig {
        rpm: None,
        tpm: Some(10),
    };
    let config = base_config(key);
    let clock = Box::new(FixedClock { now: 180 });
    let mut gateway = Gateway::with_clock(config, clock);

    let (backend, _calls) = StaticBackend::new("ok");
    gateway.register_backend("primary", backend);

    let mut request = base_request();
    request.input_tokens = 6;
    request.max_output_tokens = 5;
    let err = gateway.handle(request).await.unwrap_err();
    assert!(matches!(err, GatewayError::RateLimited { .. }));
}

#[tokio::test]
async fn cache_hit_skips_backend() {
    let mut key = base_key();
    key.cache = CacheConfig {
        enabled: true,
        ttl_seconds: Some(60),
    };
    let config = base_config(key);
    let clock = Box::new(FixedClock { now: 300 });
    let mut gateway = Gateway::with_clock(config, clock);

    let (backend, calls) = StaticBackend::new("cached");
    gateway.register_backend("primary", backend);

    let request = base_request();
    let first = gateway.handle(request.clone()).await.unwrap();
    let second = gateway.handle(request).await.unwrap();

    assert!(!first.cached);
    assert!(second.cached);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn budget_limit_blocks_request() {
    let mut key = base_key();
    key.budget = BudgetConfig {
        total_tokens: Some(5),
        total_usd_micros: None,
    };
    let config = base_config(key);
    let clock = Box::new(FixedClock { now: 360 });
    let mut gateway = Gateway::with_clock(config, clock);

    let (backend, _calls) = StaticBackend::new("ok");
    gateway.register_backend("primary", backend);

    let request = base_request();
    let err = gateway.handle(request).await.unwrap_err();
    assert!(matches!(err, GatewayError::BudgetExceeded { .. }));
}

#[tokio::test]
async fn router_switches_backend_by_model_prefix() {
    let mut config = base_config(base_key());
    config.router = RouterConfig {
        default_backend: "primary".to_string(),
        default_backends: Vec::new(),
        rules: vec![RouteRule {
            model_prefix: "gpt-4".to_string(),
            backend: "secondary".to_string(),
            backends: Vec::new(),
        }],
    };
    let clock = Box::new(FixedClock { now: 420 });
    let mut gateway = Gateway::with_clock(config, clock);

    let (primary, _primary_calls) = StaticBackend::new("primary");
    let (secondary, _secondary_calls) = StaticBackend::new("secondary");
    gateway.register_backend("primary", primary);
    gateway.register_backend("secondary", secondary);

    let request = base_request();
    let response = gateway.handle(request).await.unwrap();
    assert_eq!(response.backend, "secondary");
}

#[tokio::test]
async fn guardrail_rejects_banned_phrase() {
    let mut key = base_key();
    key.guardrails = GuardrailsConfig {
        banned_phrases: vec!["forbidden".to_string()],
        max_input_tokens: None,
    };
    let config = base_config(key);
    let clock = Box::new(FixedClock { now: 480 });
    let mut gateway = Gateway::with_clock(config, clock);

    let (backend, _calls) = StaticBackend::new("ok");
    gateway.register_backend("primary", backend);

    let mut request = base_request();
    request.prompt = "contains forbidden text".to_string();
    let err = gateway.handle(request).await.unwrap_err();
    assert!(matches!(err, GatewayError::GuardrailRejected { .. }));
}
