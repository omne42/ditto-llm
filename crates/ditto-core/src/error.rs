use std::fmt::{self, Display, Formatter};

use crate::resources::MESSAGE_CATALOG;
use i18n_kit::{Catalog, Locale, TemplateArg, render_structured_text};
use thiserror::Error;

pub use structured_text_kit::{
    CatalogArgRef, CatalogArgValueRef, StructuredText, structured_text, try_structured_text,
};
use structured_text_kit::{CatalogText, StructuredTextValidationError};

pub fn try_structured_text_from_text_args(
    key: &str,
    args: &[TemplateArg<'_>],
) -> std::result::Result<StructuredText, StructuredTextValidationError> {
    let mut message = CatalogText::try_new(key)?;
    for arg in args {
        message.try_with_value_arg(arg.name(), arg.value())?;
    }
    Ok(StructuredText::from(message))
}

fn text_detail(message: impl ToString) -> StructuredText {
    StructuredText::freeform(message.to_string())
}

fn render_with_runtime_catalog(
    locale: Locale,
    render: impl FnOnce(&dyn Catalog, Locale) -> String,
) -> String {
    MESSAGE_CATALOG
        .with_catalog(|catalog| render(catalog, locale))
        .unwrap_or_else(|error| format!("catalog initialization failed: {error}"))
}

fn default_runtime_locale() -> Locale {
    MESSAGE_CATALOG.default_locale().unwrap_or(Locale::EN_US)
}

#[cfg(test)]
fn runtime_locale_enabled(locale: Locale) -> bool {
    MESSAGE_CATALOG.locale_enabled(locale).unwrap_or(false)
}

#[derive(Debug)]
pub enum ProviderResolutionError {
    RuntimeRouteProviderMissing,
    CatalogProviderNotFound {
        provider: String,
    },
    CatalogRouteNotFound {
        provider: String,
        model: String,
        operation: String,
    },
    RuntimeRouteModelMissing,
    RuntimeRouteBaseUrlMissing,
    ProviderBaseUrlMissing,
    UnsupportedProviderClass {
        provider_hint: String,
        resolved_provider: String,
        resolved_class: String,
    },
    GenericOpenAiCompatiblePluginUnavailable,
    ProviderCapabilitiesRequireLlm {
        scope: String,
    },
    ConfiguredProviderNotFound {
        provider: String,
    },
    ConfiguredCapabilityUnknown {
        capability: String,
    },
    ConfiguredCapabilityUnsupported {
        provider: String,
        capability: String,
    },
    RuntimeRouteCapabilityUnsupported {
        provider: String,
        model: String,
        capability: String,
    },
}

impl Display for ProviderResolutionError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&render_with_runtime_catalog(
            default_runtime_locale(),
            |catalog, locale| render_provider_resolution_error(catalog, locale, self),
        ))
    }
}

impl std::error::Error for ProviderResolutionError {}

impl ProviderResolutionError {
    #[must_use]
    pub fn render(&self, locale: Locale) -> String {
        render_with_runtime_catalog(locale, |catalog, locale| {
            render_provider_resolution_error(catalog, locale, self)
        })
    }
}

#[derive(Debug, Error)]
pub enum ReferenceCatalogLoadError {
    #[error("reference catalog load failed: {source}")]
    Config {
        #[from]
        #[source]
        source: config_kit::Error,
    },
}

#[derive(Debug)]
pub enum DittoError {
    Api {
        status: reqwest::StatusCode,
        body: String,
    },
    Http(reqwest::Error),
    Io(std::io::Error),
    InvalidResponse(StructuredText),
    ProviderResolution(ProviderResolutionError),
    AuthCommand(StructuredText),
    SecretCommand(StructuredText),
    Config(StructuredText),
    Json(serde_json::Error),
}

pub type Result<T> = std::result::Result<T, DittoError>;

impl Display for DittoError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&render_with_runtime_catalog(
            default_runtime_locale(),
            |catalog, locale| render_ditto_error(catalog, locale, self),
        ))
    }
}

impl std::error::Error for DittoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Api { .. } => None,
            Self::Http(err) => Some(err),
            Self::Io(err) => Some(err),
            Self::InvalidResponse(_) => None,
            Self::ProviderResolution(err) => Some(err),
            Self::AuthCommand(_) => None,
            Self::SecretCommand(_) => None,
            Self::Config(_) => None,
            Self::Json(err) => Some(err),
        }
    }
}

impl DittoError {
    #[must_use]
    pub fn render(&self, locale: Locale) -> String {
        render_with_runtime_catalog(locale, |catalog, locale| {
            render_ditto_error(catalog, locale, self)
        })
    }

    #[must_use]
    pub fn invalid_response_text(message: impl ToString) -> Self {
        Self::InvalidResponse(text_detail(message))
    }

    #[must_use]
    pub fn auth_command_text(message: impl ToString) -> Self {
        Self::AuthCommand(text_detail(message))
    }

    #[must_use]
    pub fn provider_model_missing(subject: impl ToString, hint: impl ToString) -> Self {
        Self::InvalidResponse(structured_text!(
            "error_detail.provider.model_missing",
            "subject" => subject.to_string(),
            "hint" => hint.to_string()
        ))
    }

    #[must_use]
    pub fn provider_auth_missing(provider: impl ToString) -> Self {
        Self::Config(structured_text!(
            "error_detail.provider.auth_missing",
            "provider" => provider.to_string()
        ))
    }

    #[must_use]
    pub fn provider_base_url_invalid(
        subject: impl ToString,
        base_url: impl ToString,
        error: impl ToString,
    ) -> Self {
        Self::Config(structured_text!(
            "error_detail.provider.base_url_invalid",
            "subject" => subject.to_string(),
            "base_url" => base_url.to_string(),
            "error" => error.to_string()
        ))
    }

    #[must_use]
    pub fn builder_capability_feature_missing(
        provider: impl ToString,
        capability: impl ToString,
    ) -> Self {
        Self::InvalidResponse(structured_text!(
            "error_detail.builder.capability_feature_missing",
            "provider" => provider.to_string(),
            "capability" => capability.to_string()
        ))
    }
}

fn render_provider_resolution_error(
    catalog: &dyn Catalog,
    locale: Locale,
    error: &ProviderResolutionError,
) -> String {
    match error {
        ProviderResolutionError::RuntimeRouteProviderMissing => catalog.render_text(
            locale,
            "provider_resolution.runtime_route_provider_missing",
            &[],
        ),
        ProviderResolutionError::CatalogProviderNotFound { provider } => catalog.render_text(
            locale,
            "provider_resolution.catalog_provider_not_found",
            &[TemplateArg::new("provider", provider.as_str())],
        ),
        ProviderResolutionError::CatalogRouteNotFound {
            provider,
            model,
            operation,
        } => catalog.render_text(
            locale,
            "provider_resolution.catalog_route_not_found",
            &[
                TemplateArg::new("provider", provider.as_str()),
                TemplateArg::new("model", model.as_str()),
                TemplateArg::new("operation", operation.as_str()),
            ],
        ),
        ProviderResolutionError::RuntimeRouteModelMissing => catalog.render_text(
            locale,
            "provider_resolution.runtime_route_model_missing",
            &[],
        ),
        ProviderResolutionError::RuntimeRouteBaseUrlMissing => catalog.render_text(
            locale,
            "provider_resolution.runtime_route_base_url_missing",
            &[],
        ),
        ProviderResolutionError::ProviderBaseUrlMissing => {
            catalog.render_text(locale, "provider_resolution.provider_base_url_missing", &[])
        }
        ProviderResolutionError::UnsupportedProviderClass {
            provider_hint,
            resolved_provider,
            resolved_class,
        } => catalog.render_text(
            locale,
            "provider_resolution.unsupported_provider_class",
            &[
                TemplateArg::new("provider_hint", provider_hint.as_str()),
                TemplateArg::new("resolved_provider", resolved_provider.as_str()),
                TemplateArg::new("resolved_class", resolved_class.as_str()),
            ],
        ),
        ProviderResolutionError::GenericOpenAiCompatiblePluginUnavailable => catalog.render_text(
            locale,
            "provider_resolution.generic_openai_compatible_plugin_unavailable",
            &[],
        ),
        ProviderResolutionError::ProviderCapabilitiesRequireLlm { scope } => catalog.render_text(
            locale,
            "provider_resolution.provider_capabilities_require_llm",
            &[TemplateArg::new("scope", scope.as_str())],
        ),
        ProviderResolutionError::ConfiguredProviderNotFound { provider } => catalog.render_text(
            locale,
            "provider_resolution.configured_provider_not_found",
            &[TemplateArg::new("provider", provider.as_str())],
        ),
        ProviderResolutionError::ConfiguredCapabilityUnknown { capability } => catalog.render_text(
            locale,
            "provider_resolution.configured_capability_unknown",
            &[TemplateArg::new("capability", capability.as_str())],
        ),
        ProviderResolutionError::ConfiguredCapabilityUnsupported {
            provider,
            capability,
        } => catalog.render_text(
            locale,
            "provider_resolution.configured_capability_unsupported",
            &[
                TemplateArg::new("provider", provider.as_str()),
                TemplateArg::new("capability", capability.as_str()),
            ],
        ),
        ProviderResolutionError::RuntimeRouteCapabilityUnsupported {
            provider,
            model,
            capability,
        } => catalog.render_text(
            locale,
            "provider_resolution.runtime_route_capability_unsupported",
            &[
                TemplateArg::new("provider", provider.as_str()),
                TemplateArg::new("model", model.as_str()),
                TemplateArg::new("capability", capability.as_str()),
            ],
        ),
    }
}

fn render_ditto_error(catalog: &dyn Catalog, locale: Locale, error: &DittoError) -> String {
    match error {
        DittoError::Api { status, body } => catalog.render_text(
            locale,
            "error.api",
            &[
                TemplateArg::new("status", status.to_string()),
                TemplateArg::new("body", body.as_str()),
            ],
        ),
        DittoError::Http(err) => catalog.render_text(
            locale,
            "error.http",
            &[TemplateArg::new("error", err.to_string())],
        ),
        DittoError::Io(err) => catalog.render_text(
            locale,
            "error.io",
            &[TemplateArg::new("error", err.to_string())],
        ),
        DittoError::InvalidResponse(message) => catalog.render_text(
            locale,
            "error.invalid_response",
            &[TemplateArg::new(
                "message",
                render_structured_text(catalog, locale, message),
            )],
        ),
        DittoError::ProviderResolution(error) => {
            render_provider_resolution_error(catalog, locale, error)
        }
        DittoError::AuthCommand(message) => catalog.render_text(
            locale,
            "error.auth_command",
            &[TemplateArg::new(
                "message",
                render_structured_text(catalog, locale, message),
            )],
        ),
        DittoError::SecretCommand(message) => catalog.render_text(
            locale,
            "error.secret_command",
            &[TemplateArg::new(
                "message",
                render_structured_text(catalog, locale, message),
            )],
        ),
        DittoError::Config(message) => catalog.render_text(
            locale,
            "error.config",
            &[TemplateArg::new(
                "message",
                render_structured_text(catalog, locale, message),
            )],
        ),
        DittoError::Json(err) => catalog.render_text(
            locale,
            "error.json_parse",
            &[TemplateArg::new("error", err.to_string())],
        ),
    }
}

impl From<reqwest::Error> for DittoError {
    fn from(value: reqwest::Error) -> Self {
        Self::Http(value)
    }
}

impl From<std::io::Error> for DittoError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<ProviderResolutionError> for DittoError {
    fn from(value: ProviderResolutionError) -> Self {
        Self::ProviderResolution(value)
    }
}

impl From<serde_json::Error> for DittoError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<::secret_kit::SecretError> for DittoError {
    fn from(value: ::secret_kit::SecretError) -> Self {
        match value {
            ::secret_kit::SecretError::Io { source, .. } => Self::Io(source),
            ::secret_kit::SecretError::Json { source, .. } => Self::Json(source),
            ::secret_kit::SecretError::Lookup(message) => Self::InvalidResponse(message),
            ::secret_kit::SecretError::InvalidSpec(message) => Self::InvalidResponse(message),
            ::secret_kit::SecretError::Command(message) => Self::SecretCommand(message),
        }
    }
}

#[macro_export]
#[doc(hidden)]
macro_rules! __ditto_checked_structured_text {
    ($result:expr) => {
        match $result {
            Ok(message) => message,
            Err(error) => $crate::error::StructuredText::freeform(error.to_string()),
        }
    };
}

#[macro_export]
macro_rules! invalid_response {
    ($code:literal $(,)?) => {
        $crate::error::DittoError::InvalidResponse(
            $crate::error::structured_text!($code)
        )
    };
    ($code:literal, $($rest:tt)*) => {
        $crate::error::DittoError::InvalidResponse(
            $crate::error::structured_text!($code, $($rest)*)
        )
    };
    ($code:expr $(,)?) => {
        $crate::error::DittoError::InvalidResponse(
            $crate::__ditto_checked_structured_text!($crate::error::try_structured_text!($code))
        )
    };
    ($code:expr, $($rest:tt)*) => {
        $crate::error::DittoError::InvalidResponse(
            $crate::__ditto_checked_structured_text!(
                $crate::error::try_structured_text!($code, $($rest)*)
            )
        )
    };
}

#[macro_export]
macro_rules! auth_command_error {
    ($code:literal $(,)?) => {
        $crate::error::DittoError::AuthCommand(
            $crate::error::structured_text!($code)
        )
    };
    ($code:literal, $($rest:tt)*) => {
        $crate::error::DittoError::AuthCommand(
            $crate::error::structured_text!($code, $($rest)*)
        )
    };
    ($code:expr $(,)?) => {
        $crate::error::DittoError::AuthCommand(
            $crate::__ditto_checked_structured_text!($crate::error::try_structured_text!($code))
        )
    };
    ($code:expr, $($rest:tt)*) => {
        $crate::error::DittoError::AuthCommand(
            $crate::__ditto_checked_structured_text!(
                $crate::error::try_structured_text!($code, $($rest)*)
            )
        )
    };
}

#[macro_export]
macro_rules! config_error {
    ($code:literal $(,)?) => {
        $crate::error::DittoError::Config(
            $crate::error::structured_text!($code)
        )
    };
    ($code:literal, $($rest:tt)*) => {
        $crate::error::DittoError::Config(
            $crate::error::structured_text!($code, $($rest)*)
        )
    };
    ($code:expr $(,)?) => {
        $crate::error::DittoError::Config(
            $crate::__ditto_checked_structured_text!($crate::error::try_structured_text!($code))
        )
    };
    ($code:expr, $($rest:tt)*) => {
        $crate::error::DittoError::Config(
            $crate::__ditto_checked_structured_text!(
                $crate::error::try_structured_text!($code, $($rest)*)
            )
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use i18n_kit::{Locale, TranslationCatalog, TranslationResolution};
    use std::error::Error as _;
    use std::sync::Arc;

    struct TestFallbackCatalog;

    impl TranslationCatalog for TestFallbackCatalog {
        fn resolve_shared(&self, locale: Locale, key: &str) -> TranslationResolution {
            fallback_translation(locale, key)
                .or_else(|| fallback_translation(Locale::EN_US, key))
                .map(TranslationResolution::Exact)
                .unwrap_or(TranslationResolution::Missing)
        }
    }

    impl Catalog for TestFallbackCatalog {
        fn default_locale(&self) -> Locale {
            Locale::EN_US
        }

        fn available_locales(&self) -> Vec<Locale> {
            vec![Locale::EN_US, Locale::ZH_CN]
        }
    }

    fn fallback_translation(locale: Locale, key: &str) -> Option<Arc<str>> {
        let value = match (locale, key) {
            (Locale::EN_US, "error.config") => "config error: {message}",
            (Locale::EN_US, "error_detail.auth.header_name_empty") => {
                "auth header name must be non-empty"
            }
            (Locale::EN_US, "error_detail.provider.auth_missing") => "{provider} auth is missing",
            (Locale::EN_US, "provider_resolution.catalog_provider_not_found") => {
                "catalog provider not found: {provider}"
            }
            (Locale::ZH_CN, "error.config") => "配置错误：{message}",
            (Locale::ZH_CN, "error_detail.auth.header_name_empty") => "认证请求头名称不能为空",
            (Locale::ZH_CN, "error_detail.provider.auth_missing") => "{provider} 缺少 auth 配置",
            (Locale::ZH_CN, "provider_resolution.catalog_provider_not_found") => {
                "未找到 catalog provider：{provider}"
            }
            _ => return None,
        };

        Some(Arc::from(value))
    }

    fn render_with_test_or_runtime_catalog(
        locale: Locale,
        render: impl Fn(&dyn Catalog, Locale) -> String,
    ) -> String {
        MESSAGE_CATALOG
            .with_catalog(|catalog| render(catalog, locale))
            .unwrap_or_else(|_| {
                let catalog = TestFallbackCatalog;
                render(&catalog, locale)
            })
    }

    fn render_structured_with_runtime_catalog(locale: Locale, message: &StructuredText) -> String {
        render_with_test_or_runtime_catalog(locale, |catalog, locale| {
            render_structured_text(catalog, locale, message)
        })
    }

    fn render_provider_resolution_for_test(
        locale: Locale,
        error: &ProviderResolutionError,
    ) -> String {
        render_with_test_or_runtime_catalog(locale, |catalog, locale| {
            render_provider_resolution_error(catalog, locale, error)
        })
    }

    fn render_ditto_error_for_test(locale: Locale, error: &DittoError) -> String {
        render_with_test_or_runtime_catalog(locale, |catalog, locale| {
            render_ditto_error(catalog, locale, error)
        })
    }

    #[test]
    fn provider_config_helpers_return_config_errors() {
        assert!(matches!(
            DittoError::provider_auth_missing("vertex"),
            DittoError::Config(_)
        ));
        assert!(matches!(
            DittoError::provider_base_url_invalid("bedrock", "https://example.invalid", "bad url",),
            DittoError::Config(_)
        ));
    }

    #[test]
    fn structured_text_macro_preserves_nested_messages() {
        let message = try_structured_text!(
            "error.invalid_response",
            "message" => @text structured_text!("error_detail.auth.header_name_empty"),
        )
        .expect("nested structured message should be valid");

        let nested = message
            .as_catalog()
            .and_then(|message| message.arg("message"))
            .and_then(|arg| arg.nested_text_value())
            .and_then(|message| message.as_catalog())
            .map(|message| message.code());
        assert_eq!(nested, Some("error_detail.auth.header_name_empty"));
    }

    #[test]
    fn source_is_present_for_wrapped_errors() {
        let io_error = DittoError::from(std::io::Error::other("disk"));
        assert_eq!(
            io_error.source().map(ToString::to_string).as_deref(),
            Some("disk")
        );

        let json_error = DittoError::from(
            serde_json::from_str::<serde_json::Value>("not json").expect_err("invalid json"),
        );
        assert!(json_error.source().is_some());

        let provider_error = DittoError::from(ProviderResolutionError::RuntimeRouteModelMissing);
        assert!(provider_error.source().is_some());

        let http_error = reqwest::Client::new()
            .get("http://[::1")
            .build()
            .expect_err("invalid url should fail request construction");
        let http_error = DittoError::from(http_error);
        assert!(http_error.source().is_some());
    }

    #[test]
    fn source_is_absent_for_status_and_structured_errors() {
        let api_error = DittoError::Api {
            status: reqwest::StatusCode::BAD_REQUEST,
            body: "bad request".to_string(),
        };
        assert!(api_error.source().is_none());
        assert!(
            DittoError::InvalidResponse(text_detail("boom"))
                .source()
                .is_none()
        );
        assert!(
            DittoError::AuthCommand(text_detail("boom"))
                .source()
                .is_none()
        );
        assert!(
            DittoError::SecretCommand(text_detail("boom"))
                .source()
                .is_none()
        );
        assert!(DittoError::Config(text_detail("boom")).source().is_none());
    }

    #[test]
    fn display_uses_default_locale_rendering() {
        let error = DittoError::provider_auth_missing("vertex");
        assert_eq!(error.to_string(), error.render(default_runtime_locale()));
    }

    #[test]
    fn localized_render_helpers_use_requested_locale() {
        let message = structured_text!("error_detail.auth.header_name_empty");
        assert_eq!(
            render_structured_with_runtime_catalog(Locale::EN_US, &message),
            "auth header name must be non-empty"
        );
        assert_eq!(
            render_structured_with_runtime_catalog(Locale::ZH_CN, &message),
            "认证请求头名称不能为空"
        );

        let provider_error = ProviderResolutionError::CatalogProviderNotFound {
            provider: "acme".to_string(),
        };
        assert_eq!(
            render_provider_resolution_for_test(Locale::EN_US, &provider_error),
            "catalog provider not found: acme"
        );
        assert_eq!(
            render_provider_resolution_for_test(Locale::ZH_CN, &provider_error),
            "未找到 catalog provider：acme"
        );

        let error = DittoError::provider_auth_missing("vertex");
        assert_eq!(
            render_ditto_error_for_test(Locale::EN_US, &error),
            "config error: vertex auth is missing"
        );
        assert_eq!(
            render_ditto_error_for_test(Locale::ZH_CN, &error),
            "配置错误：vertex 缺少 auth 配置"
        );
    }

    #[test]
    fn rendering_falls_back_to_default_locale_when_catalog_is_unavailable() {
        let Some(unavailable_locale) = [Locale::EN_US, Locale::ZH_CN, Locale::JA_JP]
            .into_iter()
            .find(|locale| !runtime_locale_enabled(*locale))
        else {
            return;
        };

        let error = DittoError::provider_auth_missing("vertex");
        assert_eq!(
            error.render(unavailable_locale),
            error.render(default_runtime_locale())
        );
    }
}
