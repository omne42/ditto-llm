use std::fmt::{self, Display, Formatter};

use ::i18n::{Locale, MessageArg, MessageCatalog, MessageCatalogExt as _};
use thiserror::Error;

pub use ::error::{StructuredMessage, StructuredMessageArg, StructuredMessageValue};

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
        f.write_str(&::error::render_localized(
            self,
            &crate::MESSAGE_CATALOG,
            crate::MESSAGE_CATALOG.default_locale(),
        ))
    }
}

impl std::error::Error for ProviderResolutionError {}

impl ::error::LocalizedMessage for ProviderResolutionError {
    fn render_localized<C>(&self, catalog: &C, locale: Locale) -> String
    where
        C: MessageCatalog + ?Sized,
    {
        match self {
            Self::RuntimeRouteProviderMissing => catalog.render(
                locale,
                "provider_resolution.runtime_route_provider_missing",
                &[],
            ),
            Self::CatalogProviderNotFound { provider } => catalog.render(
                locale,
                "provider_resolution.catalog_provider_not_found",
                &[MessageArg::new("provider", provider.as_str())],
            ),
            Self::CatalogRouteNotFound {
                provider,
                model,
                operation,
            } => catalog.render(
                locale,
                "provider_resolution.catalog_route_not_found",
                &[
                    MessageArg::new("provider", provider.as_str()),
                    MessageArg::new("model", model.as_str()),
                    MessageArg::new("operation", operation.as_str()),
                ],
            ),
            Self::RuntimeRouteModelMissing => catalog.render(
                locale,
                "provider_resolution.runtime_route_model_missing",
                &[],
            ),
            Self::RuntimeRouteBaseUrlMissing => catalog.render(
                locale,
                "provider_resolution.runtime_route_base_url_missing",
                &[],
            ),
            Self::ProviderBaseUrlMissing => {
                catalog.render(locale, "provider_resolution.provider_base_url_missing", &[])
            }
            Self::UnsupportedProviderClass {
                provider_hint,
                resolved_provider,
                resolved_class,
            } => catalog.render(
                locale,
                "provider_resolution.unsupported_provider_class",
                &[
                    MessageArg::new("provider_hint", format!("{provider_hint:?}")),
                    MessageArg::new("resolved_provider", resolved_provider.as_str()),
                    MessageArg::new("resolved_class", resolved_class.as_str()),
                ],
            ),
            Self::GenericOpenAiCompatiblePluginUnavailable => catalog.render(
                locale,
                "provider_resolution.generic_openai_compatible_plugin_unavailable",
                &[],
            ),
            Self::ProviderCapabilitiesRequireLlm { scope } => catalog.render(
                locale,
                "provider_resolution.provider_capabilities_require_llm",
                &[MessageArg::new("scope", scope.as_str())],
            ),
            Self::ConfiguredProviderNotFound { provider } => catalog.render(
                locale,
                "provider_resolution.configured_provider_not_found",
                &[MessageArg::new("provider", provider.as_str())],
            ),
            Self::ConfiguredCapabilityUnknown { capability } => catalog.render(
                locale,
                "provider_resolution.configured_capability_unknown",
                &[MessageArg::new("capability", capability.as_str())],
            ),
            Self::ConfiguredCapabilityUnsupported {
                provider,
                capability,
            } => catalog.render(
                locale,
                "provider_resolution.configured_capability_unsupported",
                &[
                    MessageArg::new("provider", provider.as_str()),
                    MessageArg::new("capability", capability.as_str()),
                ],
            ),
            Self::RuntimeRouteCapabilityUnsupported {
                provider,
                model,
                capability,
            } => catalog.render(
                locale,
                "provider_resolution.runtime_route_capability_unsupported",
                &[
                    MessageArg::new("provider", provider.as_str()),
                    MessageArg::new("model", model.as_str()),
                    MessageArg::new("capability", capability.as_str()),
                ],
            ),
        }
    }
}

impl ProviderResolutionError {
    #[must_use]
    pub fn render(&self, locale: Locale) -> String {
        ::error::render_localized(self, &crate::MESSAGE_CATALOG, locale)
    }

    #[must_use]
    pub fn localized(
        &self,
        locale: Locale,
    ) -> ::error::LocalizedDisplay<'_, Self, ::i18n::StaticJsonCatalog> {
        ::error::localized(self, &crate::MESSAGE_CATALOG, locale)
    }
}

#[derive(Debug, Error)]
pub enum ReferenceCatalogLoadError {
    #[error("failed to read reference catalog {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse reference catalog JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("failed to parse reference catalog TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("unsupported reference catalog extension for {0}")]
    UnsupportedExtension(String),
}

#[derive(Debug)]
pub enum DittoError {
    Api {
        status: reqwest::StatusCode,
        body: String,
    },
    Http(reqwest::Error),
    Io(std::io::Error),
    InvalidResponse(::error::StructuredMessage),
    ProviderResolution(ProviderResolutionError),
    AuthCommand(::error::StructuredMessage),
    Config(::error::StructuredMessage),
    Json(serde_json::Error),
}

pub type Result<T> = std::result::Result<T, DittoError>;

impl Display for DittoError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&::error::render_localized(
            self,
            &crate::MESSAGE_CATALOG,
            crate::MESSAGE_CATALOG.default_locale(),
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
            Self::Config(_) => None,
            Self::Json(err) => Some(err),
        }
    }
}

impl DittoError {
    #[must_use]
    pub fn render(&self, locale: Locale) -> String {
        ::error::render_localized(self, &crate::MESSAGE_CATALOG, locale)
    }

    #[must_use]
    pub fn localized(
        &self,
        locale: Locale,
    ) -> ::error::LocalizedDisplay<'_, Self, ::i18n::StaticJsonCatalog> {
        ::error::localized(self, &crate::MESSAGE_CATALOG, locale)
    }

    #[must_use]
    pub fn invalid_response_text(message: impl ToString) -> Self {
        Self::InvalidResponse(StructuredMessage::freeform(message))
    }

    #[must_use]
    pub fn auth_command_text(message: impl ToString) -> Self {
        Self::AuthCommand(StructuredMessage::freeform(message))
    }

    #[must_use]
    pub fn config_text(message: impl ToString) -> Self {
        Self::Config(StructuredMessage::freeform(message))
    }

    #[must_use]
    pub fn provider_model_missing(subject: impl ToString, hint: impl ToString) -> Self {
        Self::InvalidResponse(
            StructuredMessage::new("error_detail.provider.model_missing")
                .arg("subject", subject)
                .arg("hint", hint),
        )
    }

    #[must_use]
    pub fn provider_auth_missing(provider: impl ToString) -> Self {
        Self::Config(
            StructuredMessage::new("error_detail.provider.auth_missing").arg("provider", provider),
        )
    }

    #[must_use]
    pub fn provider_base_url_invalid(
        subject: impl ToString,
        base_url: impl ToString,
        error: impl ToString,
    ) -> Self {
        Self::Config(
            StructuredMessage::new("error_detail.provider.base_url_invalid")
                .arg("subject", subject)
                .arg("base_url", base_url)
                .arg("error", error),
        )
    }

    #[must_use]
    pub fn builder_capability_feature_missing(
        provider: impl ToString,
        capability: impl ToString,
    ) -> Self {
        Self::InvalidResponse(
            StructuredMessage::new("error_detail.builder.capability_feature_missing")
                .arg("provider", provider)
                .arg("capability", capability),
        )
    }
}

impl ::error::LocalizedMessage for DittoError {
    fn render_localized<C>(&self, catalog: &C, locale: Locale) -> String
    where
        C: MessageCatalog + ?Sized,
    {
        match self {
            Self::Api { status, body } => catalog.render(
                locale,
                "error.api",
                &[
                    MessageArg::new("status", status.to_string()),
                    MessageArg::new("body", body.as_str()),
                ],
            ),
            Self::Http(err) => catalog.render(
                locale,
                "error.http",
                &[MessageArg::new("error", err.to_string())],
            ),
            Self::Io(err) => catalog.render(
                locale,
                "error.io",
                &[MessageArg::new("error", err.to_string())],
            ),
            Self::InvalidResponse(message) => catalog.render(
                locale,
                "error.invalid_response",
                &[MessageArg::new(
                    "message",
                    ::error::render_structured_message(catalog, locale, message),
                )],
            ),
            Self::ProviderResolution(error) => error.render_localized(catalog, locale),
            Self::AuthCommand(message) => catalog.render(
                locale,
                "error.auth_command",
                &[MessageArg::new(
                    "message",
                    ::error::render_structured_message(catalog, locale, message),
                )],
            ),
            Self::Config(message) => catalog.render(
                locale,
                "error.config",
                &[MessageArg::new(
                    "message",
                    ::error::render_structured_message(catalog, locale, message),
                )],
            ),
            Self::Json(err) => catalog.render(
                locale,
                "error.json_parse",
                &[MessageArg::new("error", err.to_string())],
            ),
        }
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

impl From<::secret::SecretError> for DittoError {
    fn from(value: ::secret::SecretError) -> Self {
        match value {
            ::secret::SecretError::Io(err) => Self::Io(err),
            ::secret::SecretError::Json(err) => Self::Json(err),
            ::secret::SecretError::InvalidSpec(message) => Self::InvalidResponse(message),
            ::secret::SecretError::AuthCommand(message) => Self::AuthCommand(message),
        }
    }
}

#[macro_export]
macro_rules! invalid_response {
    ($code:expr $(,)?) => {
        $crate::error::DittoError::InvalidResponse(
            ::error::structured_message!($code)
        )
    };
    ($code:expr, $($rest:tt)*) => {
        $crate::error::DittoError::InvalidResponse(
            ::error::structured_message!($code, $($rest)*)
        )
    };
}

#[macro_export]
macro_rules! auth_command_error {
    ($code:expr $(,)?) => {
        $crate::error::DittoError::AuthCommand(
            ::error::structured_message!($code)
        )
    };
    ($code:expr, $($rest:tt)*) => {
        $crate::error::DittoError::AuthCommand(
            ::error::structured_message!($code, $($rest)*)
        )
    };
}

#[macro_export]
macro_rules! config_error {
    ($code:expr $(,)?) => {
        $crate::error::DittoError::Config(
            ::error::structured_message!($code)
        )
    };
    ($code:expr, $($rest:tt)*) => {
        $crate::error::DittoError::Config(
            ::error::structured_message!($code, $($rest)*)
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::i18n::Locale;
    use std::error::Error as _;

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
    fn structured_message_macro_preserves_nested_messages() {
        let message = ::error::structured_message!(
            "error.invalid_response",
            "message" => @message StructuredMessage::new("error_detail.auth.header_name_empty"),
        );

        let nested = message
            .args()
            .iter()
            .find(|arg| arg.name() == "message")
            .and_then(StructuredMessageArg::message_value)
            .map(StructuredMessage::code);
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
            DittoError::InvalidResponse(::error::StructuredMessage::freeform("boom"))
                .source()
                .is_none()
        );
        assert!(
            DittoError::AuthCommand(::error::StructuredMessage::freeform("boom"))
                .source()
                .is_none()
        );
        assert!(
            DittoError::Config(::error::StructuredMessage::freeform("boom"))
                .source()
                .is_none()
        );
    }

    #[test]
    fn display_uses_default_locale_rendering() {
        let error = DittoError::provider_auth_missing("vertex");
        assert_eq!(
            error.to_string(),
            error.render(crate::MESSAGE_CATALOG.default_locale())
        );
    }

    #[cfg(feature = "i18n-en-us")]
    #[test]
    fn localized_render_helpers_use_requested_locale() {
        let message = ::error::StructuredMessage::new("error_detail.auth.header_name_empty");
        assert_eq!(
            ::error::render_structured_message(&crate::MESSAGE_CATALOG, Locale::EnUs, &message),
            "auth header name must be non-empty"
        );

        let provider_error = ProviderResolutionError::CatalogProviderNotFound {
            provider: "acme".to_string(),
        };
        assert_eq!(
            provider_error.render(Locale::EnUs),
            "catalog provider not found: acme"
        );

        let error = DittoError::provider_auth_missing("vertex");
        assert_eq!(
            error.render(Locale::EnUs),
            "config error: vertex auth is missing"
        );
        assert_eq!(
            error.localized(Locale::EnUs).to_string(),
            error.render(Locale::EnUs)
        );
    }

    #[test]
    fn rendering_falls_back_to_default_locale_when_catalog_is_unavailable() {
        let Some(unavailable_locale) = [Locale::EnUs, Locale::ZhCn, Locale::JaJp]
            .into_iter()
            .find(|locale| !crate::MESSAGE_CATALOG.locale_enabled(*locale))
        else {
            return;
        };

        let error = DittoError::provider_auth_missing("vertex");
        assert_eq!(
            error.render(unavailable_locale),
            error.render(crate::MESSAGE_CATALOG.default_locale())
        );
    }
}
