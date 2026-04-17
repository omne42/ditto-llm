use std::collections::{BTreeMap, HashMap};

use async_trait::async_trait;
#[cfg(feature = "cap-llm-streaming")]
use futures_util::StreamExt;
#[cfg(feature = "cap-llm-streaming")]
use futures_util::stream;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::genai;
use crate::config::{
    Env, HttpAuth, ProviderConfig, RequestAuth, resolve_provider_request_auth_required,
};
use crate::llm_core::model::{LanguageModel, StreamResult};
use crate::provider_transport::{
    DEFAULT_HTTP_TIMEOUT, apply_http_query_params, default_http_client,
    resolve_http_provider_config,
};
#[cfg(feature = "cap-llm-streaming")]
use crate::contracts::StreamChunk;
use crate::contracts::{
    ContentPart, FinishReason, GenerateRequest, GenerateResponse, Message, Tool, ToolChoice, Usage,
    Warning,
};
use crate::error::{DittoError, Result};

#[cfg(feature = "cap-embedding")]
use crate::capabilities::embedding::EmbeddingModel;

const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

#[derive(Clone)]
pub struct Google {
    http: reqwest::Client,
    base_url: String,
    auth: Option<RequestAuth>,
    default_model: String,
    http_query_params: BTreeMap<String, String>,
}

impl Google {
    pub fn new(api_key: impl Into<String>) -> Self {
        let http = default_http_client(DEFAULT_HTTP_TIMEOUT);

        let api_key = api_key.into();
        let auth = if api_key.trim().is_empty() {
            None
        } else {
            HttpAuth::header_value("x-goog-api-key", None, &api_key)
                .ok()
                .map(RequestAuth::Http)
        };

        Self {
            http,
            base_url: DEFAULT_BASE_URL.to_string(),
            auth,
            default_model: String::new(),
            http_query_params: BTreeMap::new(),
        }
    }

    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.http = http;
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = model.into();
        self
    }

    pub async fn from_config(config: &ProviderConfig, env: &Env) -> Result<Self> {
        const DEFAULT_KEYS: &[&str] = &["GOOGLE_API_KEY", "GEMINI_API_KEY"];
        let auth_header = resolve_provider_request_auth_required(
            config,
            env,
            DEFAULT_KEYS,
            "x-goog-api-key",
            None,
        )
        .await?;
        let resolved =
            resolve_http_provider_config(DEFAULT_HTTP_TIMEOUT, config, Some(DEFAULT_BASE_URL))?;

        let mut out = Self::new("").with_http_client(resolved.http);
        out.auth = Some(auth_header);
        out.http_query_params = resolved.http_query_params;
        if let Some(base_url) = resolved.base_url {
            out = out.with_base_url(base_url);
        }
        if let Some(model) = resolved.default_model {
            out = out.with_model(model);
        }
        Ok(out)
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let req = match self.auth.as_ref() {
            Some(auth) => auth.apply(req),
            None => req,
        };
        apply_http_query_params(req, &self.http_query_params)
    }

    fn resolve_model<'a>(&'a self, request: &'a GenerateRequest) -> Result<&'a str> {
        crate::providers::resolve_model_or_default(
            request.model.as_deref().filter(|m| !m.trim().is_empty()),
            self.default_model.as_str(),
            "google",
            "set request.model or Google::with_model",
        )
    }

    fn model_path(model: &str) -> String {
        let model = model.trim();
        if model.starts_with("models/") {
            model.to_string()
        } else {
            format!("models/{model}")
        }
    }

    fn generate_url(&self, model: &str) -> String {
        let path = Self::model_path(model);
        http_kit::join_api_base_url_path(&self.base_url, &format!("{path}:generateContent"))
    }

    #[cfg(feature = "cap-llm-streaming")]
    fn stream_url(&self, model: &str) -> String {
        let path = Self::model_path(model);
        http_kit::append_url_query_params(
            http_kit::join_api_base_url_path(&self.base_url, &format!("{path}:streamGenerateContent")),
            &[("alt".to_string(), "sse".to_string())],
        )
    }

    fn build_tool_name_map(messages: &[Message]) -> HashMap<String, String> {
        genai::build_tool_name_map(messages)
    }

    fn convert_messages(
        model: &str,
        messages: &[Message],
        tool_names: &HashMap<String, String>,
        warnings: &mut Vec<Warning>,
    ) -> Result<(Vec<Value>, Option<Value>)> {
        genai::convert_messages(model, messages, tool_names, warnings)
    }

    fn tool_to_google(tool: Tool, warnings: &mut Vec<Warning>) -> Value {
        genai::tool_to_google(tool, warnings)
    }

    fn tool_config(choice: Option<&ToolChoice>) -> Option<Value> {
        genai::tool_config(choice)
    }

    fn map_finish_reason(finish_reason: Option<&str>, has_tool_calls: bool) -> FinishReason {
        genai::map_finish_reason(finish_reason, has_tool_calls)
    }

    fn parse_usage_metadata(value: &Value) -> Usage {
        genai::parse_usage_metadata(value)
    }
}
