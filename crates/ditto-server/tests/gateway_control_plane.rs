#![cfg(feature = "gateway")]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;

use ditto_server::gateway::{
    Backend, BackendConfig, BudgetConfig, CacheConfig, Clock, Gateway, GatewayConfig, GatewayError,
    GatewayRequest, GatewayResponse, GuardrailsConfig, LimitsConfig, PassthroughConfig,
    RouteBackend, RouteRule, RouterConfig, VirtualKeyConfig,
};

struct FixedClock {
    now: u64,
}

impl Clock for FixedClock {
    fn now_epoch_seconds(&self) -> u64 {
        self.now
    }
}

struct ScriptedClock {
    times: Vec<u64>,
    idx: AtomicUsize,
}

impl ScriptedClock {
    fn new(times: Vec<u64>) -> Self {
        Self {
            times,
            idx: AtomicUsize::new(0),
        }
    }
}

impl Clock for ScriptedClock {
    fn now_epoch_seconds(&self) -> u64 {
        let idx = self.idx.fetch_add(1, Ordering::SeqCst);
        self.times
            .get(idx)
            .copied()
            .or_else(|| self.times.last().copied())
            .unwrap_or(0)
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
        tenant_id: None,
        project_id: None,
        user_id: None,
        tenant_budget: None,
        project_budget: None,
        user_budget: None,
        tenant_limits: None,
        project_limits: None,
        user_limits: None,
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
            default_backends: vec![RouteBackend {
                backend: "primary".to_string(),
                weight: 1.0,
            }],
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
        observability: Default::default(),
    }
}

fn provider_backend_config(name: &str, provider: &str) -> BackendConfig {
    BackendConfig {
        name: name.to_string(),
        base_url: String::new(),
        max_in_flight: None,
        timeout_seconds: None,
        headers: Default::default(),
        query_params: Default::default(),
        provider: Some(provider.to_string()),
        provider_config: None,
        model_map: Default::default(),
    }
}

#[tokio::test]
async fn virtual_key_validation_rejects_unknown() {
    let config = base_config(base_key());
    let clock = Box::new(FixedClock { now: 0 });
    let gateway = Gateway::with_clock(config, clock);

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
async fn project_rate_limit_rpm_blocks_second_request() {
    let mut key = base_key();
    key.project_id = Some("project-1".to_string());
    key.project_limits = Some(LimitsConfig {
        rpm: Some(1),
        tpm: None,
    });
    let config = base_config(key);
    let clock = Box::new(FixedClock { now: 240 });
    let mut gateway = Gateway::with_clock(config, clock);

    let (backend, _calls) = StaticBackend::new("ok");
    gateway.register_backend("primary", backend);

    let request = base_request();
    gateway.handle(request.clone()).await.unwrap();
    let err = gateway.handle(request).await.unwrap_err();
    assert!(matches!(err, GatewayError::RateLimited { .. }));
}

#[tokio::test]
async fn user_rate_limit_tpm_blocks_second_request() {
    let mut key = base_key();
    key.user_id = Some("user-1".to_string());
    key.user_limits = Some(LimitsConfig {
        rpm: None,
        tpm: Some(10),
    });
    let config = base_config(key);
    let clock = Box::new(FixedClock { now: 240 });
    let mut gateway = Gateway::with_clock(config, clock);

    let (backend, _calls) = StaticBackend::new("ok");
    gateway.register_backend("primary", backend);

    let mut request = base_request();
    request.input_tokens = 5;
    request.max_output_tokens = 5;
    gateway.handle(request.clone()).await.unwrap();
    request.input_tokens = 1;
    request.max_output_tokens = 5;
    let err = gateway.handle(request).await.unwrap_err();
    assert!(matches!(err, GatewayError::RateLimited { .. }));
}

#[tokio::test]
async fn rate_limit_rejection_does_not_consume_other_scopes() {
    let mut key = base_key();
    key.tenant_id = Some("tenant-1".to_string());
    key.user_id = Some("user-1".to_string());
    key.limits = LimitsConfig {
        rpm: Some(1),
        tpm: None,
    };
    key.tenant_limits = Some(LimitsConfig {
        rpm: Some(1),
        tpm: None,
    });
    key.user_limits = Some(LimitsConfig {
        rpm: None,
        tpm: Some(5),
    });
    let config = base_config(key);
    let clock = Box::new(FixedClock { now: 240 });
    let mut gateway = Gateway::with_clock(config, clock);

    let (backend, calls) = StaticBackend::new("ok");
    gateway.register_backend("primary", backend);

    let request = base_request();
    let err = gateway.handle(request.clone()).await.unwrap_err();
    assert!(matches!(err, GatewayError::RateLimited { .. }));

    let mut recovered = request;
    recovered.input_tokens = 2;
    recovered.max_output_tokens = 2;
    let response = gateway.handle(recovered).await.unwrap();
    assert_eq!(response.content, "ok");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cache_hit_skips_backend() {
    let mut key = base_key();
    key.cache = CacheConfig {
        enabled: true,
        ttl_seconds: Some(60),
        ..CacheConfig::default()
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
async fn cache_ttl_starts_from_response_write_time() {
    let mut key = base_key();
    key.cache = CacheConfig {
        enabled: true,
        ttl_seconds: Some(60),
        ..CacheConfig::default()
    };
    let config = base_config(key);
    let clock = Box::new(ScriptedClock::new(vec![0, 120, 120]));
    let mut gateway = Gateway::with_clock(config, clock);

    let (backend, calls) = StaticBackend::new("cached");
    gateway.register_backend("primary", backend);

    let request = base_request();
    gateway.handle(request.clone()).await.unwrap();
    gateway.handle(request).await.unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn upsert_virtual_key_cache_config_change_clears_stale_scope_cache() {
    let mut key = base_key();
    key.cache = CacheConfig {
        enabled: true,
        ttl_seconds: Some(60),
        ..CacheConfig::default()
    };
    let config = base_config(key);
    let clock = Box::new(FixedClock { now: 300 });
    let mut gateway = Gateway::with_clock(config, clock);

    let (backend, calls) = StaticBackend::new("cached");
    gateway.register_backend("primary", backend);

    let request = base_request();
    let first = gateway.handle(request.clone()).await.unwrap();
    let second = gateway.handle(request.clone()).await.unwrap();
    assert!(!first.cached);
    assert!(second.cached);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let mut updated = base_key();
    updated.cache = CacheConfig {
        enabled: true,
        ttl_seconds: Some(5),
        max_entries: 1,
        ..CacheConfig::default()
    };
    assert!(!gateway.upsert_virtual_key(updated));

    let third = gateway.handle(request.clone()).await.unwrap();
    let fourth = gateway.handle(request).await.unwrap();
    assert!(!third.cached);
    assert!(fourth.cached);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn replace_virtual_keys_cache_config_change_clears_stale_scope_cache() {
    let mut key = base_key();
    key.cache = CacheConfig {
        enabled: true,
        ttl_seconds: Some(60),
        ..CacheConfig::default()
    };
    let config = base_config(key);
    let clock = Box::new(FixedClock { now: 300 });
    let mut gateway = Gateway::with_clock(config, clock);

    let (backend, calls) = StaticBackend::new("cached");
    gateway.register_backend("primary", backend);

    let request = base_request();
    let first = gateway.handle(request.clone()).await.unwrap();
    let second = gateway.handle(request.clone()).await.unwrap();
    assert!(!first.cached);
    assert!(second.cached);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let mut updated = base_key();
    updated.cache = CacheConfig {
        enabled: true,
        ttl_seconds: Some(5),
        max_entries: 1,
        ..CacheConfig::default()
    };
    gateway
        .try_replace_virtual_keys(vec![updated])
        .expect("replace should validate");

    let third = gateway.handle(request.clone()).await.unwrap();
    let fourth = gateway.handle(request).await.unwrap();
    assert!(!third.cached);
    assert!(fourth.cached);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn try_upsert_virtual_key_rejects_invalid_route_without_mutating_runtime() {
    let config = base_config(base_key());
    let clock = Box::new(FixedClock { now: 420 });
    let mut gateway = Gateway::with_clock(config, clock);

    let (backend, calls) = StaticBackend::new("ok");
    gateway.register_backend("primary", backend);

    let mut invalid = VirtualKeyConfig::new("key-2", "vk-2");
    invalid.route = Some("missing".to_string());
    let err = gateway.try_upsert_virtual_key(invalid).unwrap_err();
    assert!(matches!(err, GatewayError::InvalidRequest { .. }));

    assert_eq!(gateway.list_virtual_keys().len(), 1);
    let response = gateway.handle(base_request()).await.unwrap();
    assert_eq!(response.content, "ok");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn try_replace_virtual_keys_rejects_invalid_keys_without_mutating_runtime() {
    let config = base_config(base_key());
    let clock = Box::new(FixedClock { now: 420 });
    let gateway = Gateway::with_clock(config, clock);

    let mut invalid = base_key();
    invalid.route = Some("missing".to_string());
    let err = gateway.try_replace_virtual_keys(vec![invalid]).unwrap_err();
    assert!(matches!(err, GatewayError::InvalidRequest { .. }));

    assert_eq!(gateway.list_virtual_keys().len(), 1);
    assert_eq!(gateway.list_virtual_keys()[0].token.as_str(), "vk-1");
}

#[tokio::test]
async fn try_replace_router_config_rejects_invalid_router_without_mutating_runtime() {
    let config = base_config(base_key());
    let clock = Box::new(FixedClock { now: 420 });
    let gateway = Gateway::with_clock(config, clock);

    let invalid = RouterConfig {
        default_backends: vec![RouteBackend {
            backend: "missing".to_string(),
            weight: 1.0,
        }],
        rules: Vec::new(),
    };
    let err = gateway.try_replace_router_config(invalid).unwrap_err();
    assert!(matches!(err, GatewayError::InvalidRequest { .. }));

    assert_eq!(
        gateway.router_config().default_backends[0].backend,
        "primary"
    );
}

#[test]
fn gateway_config_validate_rejects_unknown_router_backend_before_runtime() {
    let mut config = base_config(base_key());
    config.router = RouterConfig {
        default_backends: vec![RouteBackend {
            backend: "missing".to_string(),
            weight: 1.0,
        }],
        rules: Vec::new(),
    };

    let err = config.validate().unwrap_err();
    assert!(matches!(err, GatewayError::InvalidRequest { .. }));
    assert!(
        err.to_string()
            .contains("router references unknown backends: missing")
    );
}

#[test]
fn gateway_config_validate_accepts_provider_backends_as_router_targets() {
    let mut config = base_config(base_key());
    config.backends = vec![provider_backend_config(
        "translation-primary",
        "openai-compatible",
    )];
    config.router = RouterConfig {
        default_backends: vec![RouteBackend {
            backend: "translation-primary".to_string(),
            weight: 1.0,
        }],
        rules: Vec::new(),
    };
    config.virtual_keys[0].route = Some("translation-primary".to_string());

    config
        .validate()
        .expect("provider-backed translation routes should validate before runtime");
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
async fn project_budget_limit_blocks_request() {
    let mut key = base_key();
    key.project_id = Some("project-1".to_string());
    key.project_budget = Some(BudgetConfig {
        total_tokens: Some(5),
        total_usd_micros: None,
    });
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
async fn project_budget_is_shared_across_virtual_keys() {
    let mut key_1 = base_key();
    key_1.project_id = Some("project-1".to_string());
    key_1.project_budget = Some(BudgetConfig {
        total_tokens: Some(10),
        total_usd_micros: None,
    });

    let mut key_2 = VirtualKeyConfig::new("key-2", "vk-2");
    key_2.project_id = Some("project-1".to_string());
    key_2.project_budget = Some(BudgetConfig {
        total_tokens: Some(10),
        total_usd_micros: None,
    });

    let config = GatewayConfig {
        backends: Vec::new(),
        virtual_keys: vec![key_1, key_2],
        router: RouterConfig {
            default_backends: vec![RouteBackend {
                backend: "primary".to_string(),
                weight: 1.0,
            }],
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
        observability: Default::default(),
    };
    let clock = Box::new(FixedClock { now: 360 });
    let mut gateway = Gateway::with_clock(config, clock);

    let (backend, _calls) = StaticBackend::new("ok");
    gateway.register_backend("primary", backend);

    gateway.handle(base_request()).await.unwrap();

    let mut request = base_request();
    request.virtual_key = "vk-2".to_string();
    let err = gateway.handle(request).await.unwrap_err();
    assert!(matches!(err, GatewayError::BudgetExceeded { .. }));
}

#[tokio::test]
async fn project_budget_is_isolated_by_tenant_namespace() {
    let mut key_1 = base_key();
    key_1.tenant_id = Some("tenant-1".to_string());
    key_1.project_id = Some("project-1".to_string());
    key_1.project_budget = Some(BudgetConfig {
        total_tokens: Some(15),
        total_usd_micros: None,
    });

    let mut key_2 = VirtualKeyConfig::new("key-2", "vk-2");
    key_2.tenant_id = Some("tenant-2".to_string());
    key_2.project_id = Some("project-1".to_string());
    key_2.project_budget = Some(BudgetConfig {
        total_tokens: Some(15),
        total_usd_micros: None,
    });

    let config = GatewayConfig {
        backends: Vec::new(),
        virtual_keys: vec![key_1, key_2],
        router: RouterConfig {
            default_backends: vec![RouteBackend {
                backend: "primary".to_string(),
                weight: 1.0,
            }],
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
        observability: Default::default(),
    };
    let clock = Box::new(FixedClock { now: 360 });
    let mut gateway = Gateway::with_clock(config, clock);

    let (backend, calls) = StaticBackend::new("ok");
    gateway.register_backend("primary", backend);

    gateway.handle(base_request()).await.unwrap();

    let mut request = base_request();
    request.virtual_key = "vk-2".to_string();
    gateway.handle(request).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn user_budget_limit_blocks_request() {
    let mut key = base_key();
    key.user_id = Some("user-1".to_string());
    key.user_budget = Some(BudgetConfig {
        total_tokens: Some(5),
        total_usd_micros: None,
    });
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
async fn user_budget_is_isolated_by_tenant_namespace() {
    let mut key_1 = base_key();
    key_1.tenant_id = Some("tenant-1".to_string());
    key_1.user_id = Some("user-1".to_string());
    key_1.user_budget = Some(BudgetConfig {
        total_tokens: Some(5),
        total_usd_micros: None,
    });

    let mut key_2 = VirtualKeyConfig::new("key-2", "vk-2");
    key_2.tenant_id = Some("tenant-2".to_string());
    key_2.user_id = Some("user-1".to_string());
    key_2.user_budget = Some(BudgetConfig {
        total_tokens: Some(5),
        total_usd_micros: None,
    });

    let config = GatewayConfig {
        backends: Vec::new(),
        virtual_keys: vec![key_1, key_2],
        router: RouterConfig {
            default_backends: vec![RouteBackend {
                backend: "primary".to_string(),
                weight: 1.0,
            }],
            rules: Vec::new(),
        },
        a2a_agents: Vec::new(),
        mcp_servers: Vec::new(),
        observability: Default::default(),
    };
    let clock = Box::new(FixedClock { now: 360 });
    let mut gateway = Gateway::with_clock(config, clock);

    let (backend, calls) = StaticBackend::new("ok");
    gateway.register_backend("primary", backend);

    gateway.handle(base_request()).await.unwrap();

    let mut request = base_request();
    request.virtual_key = "vk-2".to_string();
    gateway.handle(request).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn router_switches_backend_by_model_prefix() {
    let mut config = base_config(base_key());
    config.router = RouterConfig {
        default_backends: vec![RouteBackend {
            backend: "primary".to_string(),
            weight: 1.0,
        }],
        rules: vec![RouteRule {
            model_prefix: "gpt-4".to_string(),
            exact: false,
            backend: "secondary".to_string(),
            backends: Vec::new(),
            guardrails: None,
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
async fn cache_key_does_not_cross_backend_routes() {
    let mut key = base_key();
    key.cache = CacheConfig {
        enabled: true,
        ttl_seconds: Some(60),
        ..CacheConfig::default()
    };

    let mut config = base_config(key);
    config.router = RouterConfig {
        default_backends: vec![RouteBackend {
            backend: "primary".to_string(),
            weight: 1.0,
        }],
        rules: vec![RouteRule {
            model_prefix: "gpt-4".to_string(),
            exact: false,
            backend: "secondary".to_string(),
            backends: Vec::new(),
            guardrails: None,
        }],
    };

    let clock = Box::new(FixedClock { now: 420 });
    let mut gateway = Gateway::with_clock(config, clock);

    let (primary, primary_calls) = StaticBackend::new("primary");
    let (secondary, secondary_calls) = StaticBackend::new("secondary");
    gateway.register_backend("primary", primary);
    gateway.register_backend("secondary", secondary);

    let primary_request = GatewayRequest {
        model: "o1-mini".to_string(),
        ..base_request()
    };
    let primary_first = gateway.handle(primary_request.clone()).await.unwrap();
    let primary_second = gateway.handle(primary_request).await.unwrap();
    assert_eq!(primary_first.backend, "primary");
    assert_eq!(primary_second.backend, "primary");
    assert!(!primary_first.cached);
    assert!(primary_second.cached);

    let secondary_request = base_request();
    let secondary_first = gateway.handle(secondary_request.clone()).await.unwrap();
    let secondary_second = gateway.handle(secondary_request).await.unwrap();
    assert_eq!(secondary_first.backend, "secondary");
    assert_eq!(secondary_second.backend, "secondary");
    assert!(!secondary_first.cached);
    assert!(secondary_second.cached);

    assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(secondary_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn guardrail_override_applies_for_route_rule() {
    let mut key = base_key();
    key.guardrails = GuardrailsConfig {
        banned_phrases: vec!["forbidden".to_string()],
        banned_regexes: Vec::new(),
        block_pii: false,
        validate_schema: false,
        max_input_tokens: None,
        allow_models: Vec::new(),
        deny_models: Vec::new(),
    };

    let mut config = base_config(key);
    config.router = RouterConfig {
        default_backends: vec![RouteBackend {
            backend: "primary".to_string(),
            weight: 1.0,
        }],
        rules: vec![RouteRule {
            model_prefix: "gpt-".to_string(),
            exact: false,
            backend: "primary".to_string(),
            backends: Vec::new(),
            guardrails: Some(GuardrailsConfig::default()),
        }],
    };

    let clock = Box::new(FixedClock { now: 470 });
    let mut gateway = Gateway::with_clock(config, clock);

    let (backend, _calls) = StaticBackend::new("ok");
    gateway.register_backend("primary", backend);

    let mut request = base_request();
    request.prompt = "contains forbidden text".to_string();
    let response = gateway.handle(request).await.unwrap();
    assert_eq!(response.backend, "primary");

    let mut request = base_request();
    request.model = "o1".to_string();
    request.prompt = "contains forbidden text".to_string();
    let err = gateway.handle(request).await.unwrap_err();
    assert!(matches!(err, GatewayError::GuardrailRejected { .. }));
}

#[tokio::test]
async fn guardrail_rejects_banned_phrase() {
    let mut key = base_key();
    key.guardrails = GuardrailsConfig {
        banned_phrases: vec!["forbidden".to_string()],
        banned_regexes: Vec::new(),
        block_pii: false,
        validate_schema: false,
        max_input_tokens: None,
        allow_models: Vec::new(),
        deny_models: Vec::new(),
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

#[tokio::test]
async fn guardrail_rejects_banned_regex() {
    let mut key = base_key();
    key.guardrails = GuardrailsConfig {
        banned_phrases: Vec::new(),
        banned_regexes: vec!["forbidden".to_string()],
        block_pii: false,
        validate_schema: false,
        max_input_tokens: None,
        allow_models: Vec::new(),
        deny_models: Vec::new(),
    };
    let config = base_config(key);
    let clock = Box::new(FixedClock { now: 490 });
    let mut gateway = Gateway::with_clock(config, clock);

    let (backend, _calls) = StaticBackend::new("ok");
    gateway.register_backend("primary", backend);

    let mut request = base_request();
    request.prompt = "contains forbidden text".to_string();
    let err = gateway.handle(request).await.unwrap_err();
    assert!(matches!(err, GatewayError::GuardrailRejected { .. }));
}

#[tokio::test]
async fn guardrail_rejects_pii() {
    let mut key = base_key();
    key.guardrails = GuardrailsConfig {
        banned_phrases: Vec::new(),
        banned_regexes: Vec::new(),
        block_pii: true,
        validate_schema: false,
        max_input_tokens: None,
        allow_models: Vec::new(),
        deny_models: Vec::new(),
    };
    let config = base_config(key);
    let clock = Box::new(FixedClock { now: 495 });
    let mut gateway = Gateway::with_clock(config, clock);

    let (backend, _calls) = StaticBackend::new("ok");
    gateway.register_backend("primary", backend);

    let mut request = base_request();
    request.prompt = "email test@example.com".to_string();
    let err = gateway.handle(request).await.unwrap_err();
    assert!(matches!(err, GatewayError::GuardrailRejected { .. }));
}

#[tokio::test]
async fn guardrail_rejects_denied_model() {
    let mut key = base_key();
    key.guardrails = GuardrailsConfig {
        banned_phrases: Vec::new(),
        banned_regexes: Vec::new(),
        block_pii: false,
        validate_schema: false,
        max_input_tokens: None,
        allow_models: Vec::new(),
        deny_models: vec!["gpt-4o-mini".to_string()],
    };
    let config = base_config(key);
    let clock = Box::new(FixedClock { now: 500 });
    let mut gateway = Gateway::with_clock(config, clock);

    let (backend, _calls) = StaticBackend::new("ok");
    gateway.register_backend("primary", backend);

    let request = base_request();
    let err = gateway.handle(request).await.unwrap_err();
    assert!(matches!(err, GatewayError::GuardrailRejected { .. }));
}

#[tokio::test]
async fn guardrail_rejects_not_allowed_model() {
    let mut key = base_key();
    key.guardrails = GuardrailsConfig {
        banned_phrases: Vec::new(),
        banned_regexes: Vec::new(),
        block_pii: false,
        validate_schema: false,
        max_input_tokens: None,
        allow_models: vec!["gpt-*".to_string()],
        deny_models: Vec::new(),
    };
    let config = base_config(key);
    let clock = Box::new(FixedClock { now: 520 });
    let mut gateway = Gateway::with_clock(config, clock);

    let (backend, _calls) = StaticBackend::new("ok");
    gateway.register_backend("primary", backend);

    let mut request = base_request();
    request.model = "o1".to_string();
    let err = gateway.handle(request).await.unwrap_err();
    assert!(matches!(err, GatewayError::GuardrailRejected { .. }));
}
