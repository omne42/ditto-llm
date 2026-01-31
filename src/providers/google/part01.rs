use std::collections::{BTreeMap, HashMap, VecDeque};

use async_trait::async_trait;
use futures_util::StreamExt;
use futures_util::stream;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::genai;
use crate::model::{LanguageModel, StreamResult};
use crate::profile::{
    Env, HttpAuth, ProviderAuth, ProviderConfig, RequestAuth, apply_http_query_params,
    resolve_request_auth_with_default_keys,
};
use crate::types::{
    ContentPart, FinishReason, GenerateRequest, GenerateResponse, Message, StreamChunk, Tool,
    ToolChoice, Usage, Warning,
};
use crate::{DittoError, Result};

#[cfg(feature = "embeddings")]
use crate::embedding::EmbeddingModel;

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
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

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
        const DEFAULT_KEYS: &[&str] =
            &["GOOGLE_API_KEY", "GEMINI_API_KEY", "CODE_PM_GOOGLE_API_KEY"];
        let auth = config
            .auth
            .clone()
            .unwrap_or(ProviderAuth::ApiKeyEnv { keys: Vec::new() });
        let auth_header = resolve_request_auth_with_default_keys(
            &auth,
            env,
            DEFAULT_KEYS,
            "x-goog-api-key",
            None,
        )
        .await?;

        let mut out = Self::new("");
        out.auth = Some(auth_header);
        out.http_query_params = config.http_query_params.clone();
        if !config.http_headers.is_empty() {
            out = out.with_http_client(crate::profile::build_http_client(
                std::time::Duration::from_secs(300),
                &config.http_headers,
            )?);
        }
        if let Some(base_url) = config.base_url.as_deref().filter(|s| !s.trim().is_empty()) {
            out = out.with_base_url(base_url);
        }
        if let Some(model) = config
            .default_model
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
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
        if let Some(model) = request.model.as_deref().filter(|m| !m.trim().is_empty()) {
            return Ok(model);
        }
        if !self.default_model.trim().is_empty() {
            return Ok(self.default_model.as_str());
        }
        Err(DittoError::InvalidResponse(
            "google model is not set (set request.model or Google::with_model)".to_string(),
        ))
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
        let base = self.base_url.trim_end_matches('/');
        let path = Self::model_path(model);
        format!("{base}/{path}:generateContent")
    }

    fn stream_url(&self, model: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        let path = Self::model_path(model);
        format!("{base}/{path}:streamGenerateContent?alt=sse")
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

