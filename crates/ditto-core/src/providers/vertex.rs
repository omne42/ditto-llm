use std::collections::BTreeMap;

use async_trait::async_trait;
#[cfg(feature = "cap-llm-streaming")]
use futures_util::StreamExt;
#[cfg(feature = "cap-llm-streaming")]
use futures_util::stream;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::genai;
use crate::auth::oauth::{OAuthClientCredentials, resolve_oauth_client_credentials};
use crate::config::{Env, HttpAuth, ProviderConfig};
#[cfg(feature = "cap-llm-streaming")]
#[allow(unused_imports)]
use crate::contracts::StreamChunk;
#[cfg(feature = "cap-llm-streaming")]
use crate::contracts::Usage;
use crate::contracts::Warning;
use crate::contracts::{ContentPart, GenerateRequest, GenerateResponse};
use crate::error::{DittoError, Result};
use crate::llm_core::model::{LanguageModel, StreamResult};
use crate::provider_transport::{
    DEFAULT_HTTP_TIMEOUT, build_http_client, resolve_http_provider_config,
};

#[derive(Clone)]
pub struct Vertex {
    http: reqwest::Client,
    base_url: String,
    default_model: String,
    oauth: OAuthClientCredentials,
    http_headers: BTreeMap<String, String>,
    http_query_params: BTreeMap<String, String>,
}

impl Vertex {
    pub fn new(
        oauth: OAuthClientCredentials,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Result<Self> {
        let http = build_http_client(DEFAULT_HTTP_TIMEOUT, &BTreeMap::new())?;
        Ok(Self {
            http,
            base_url: base_url.into(),
            default_model: default_model.into(),
            oauth,
            http_headers: BTreeMap::new(),
            http_query_params: BTreeMap::new(),
        })
    }

    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.http = http;
        self
    }

    pub fn with_http_headers(mut self, headers: BTreeMap<String, String>) -> Self {
        self.http_headers = headers;
        self
    }

    pub fn with_http_query_params(mut self, params: BTreeMap<String, String>) -> Self {
        self.http_query_params = params;
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
        let resolved = resolve_http_provider_config(DEFAULT_HTTP_TIMEOUT, config, None)?;
        let base_url = resolved.required_base_url()?;
        let model = resolved.required_default_model()?;
        let auth = config
            .auth
            .clone()
            .ok_or_else(|| DittoError::provider_auth_missing("vertex"))?;
        let oauth = resolve_oauth_client_credentials(&auth, env)?;

        let mut out = Self::new(oauth, base_url, model)?;
        out.http_headers = config.http_headers.clone();
        out.http_query_params = resolved.http_query_params;
        Ok(out)
    }

    fn resolve_model<'a>(&'a self, request: &'a GenerateRequest) -> Result<&'a str> {
        if let Some(model) = request.model.as_deref().filter(|m| !m.trim().is_empty()) {
            return Ok(model);
        }
        if !self.default_model.trim().is_empty() {
            return Ok(self.default_model.as_str());
        }
        Err(DittoError::provider_model_missing(
            "vertex",
            "set request.model or Vertex::with_model",
        ))
    }

    fn generate_url(&self, model: &str) -> String {
        if self.base_url.contains("{model}") {
            return self.base_url.replace("{model}", model);
        }
        if self.base_url.ends_with(":generateContent") {
            return self.base_url.clone();
        }
        http_kit::join_api_base_url_path(&self.base_url, &format!("models/{model}:generateContent"))
    }

    #[cfg(feature = "cap-llm-streaming")]
    fn stream_url(&self, model: &str) -> String {
        if self.base_url.contains("{model}") {
            let replaced = self.base_url.replace("{model}", model);
            if replaced.ends_with(":generateContent") {
                return replaced.replace(":generateContent", ":streamGenerateContent");
            }
            if replaced.ends_with(":streamGenerateContent") {
                return replaced;
            }
            return replaced;
        }
        if self.base_url.ends_with(":streamGenerateContent") {
            return self.base_url.clone();
        }
        if self.base_url.ends_with(":generateContent") {
            return self
                .base_url
                .replace(":generateContent", ":streamGenerateContent");
        }
        http_kit::join_api_base_url_path(
            &self.base_url,
            &format!("models/{model}:streamGenerateContent"),
        )
    }

    fn build_url_with_query_and_alt(&self, base: &str, alt: Option<&str>) -> Result<String> {
        let mut query_params =
            Vec::with_capacity(self.http_query_params.len() + usize::from(alt.is_some()));
        if let Some(alt) = alt {
            query_params.push(("alt".to_string(), alt.to_string()));
        }
        query_params.extend(
            self.http_query_params
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
        if query_params.is_empty() {
            return Ok(base.to_string());
        }

        http_kit::append_url_query_params_encoded(base, &query_params)
            .map_err(|err| DittoError::provider_base_url_invalid("vertex", base, err))
    }

    async fn apply_auth(&self, req: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder> {
        let token = self.oauth.fetch_token(&self.http).await?;
        let auth = HttpAuth::header_value("authorization", Some("Bearer "), &token.access_token)?;
        Ok(auth.apply(req))
    }

    fn apply_headers(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        for (name, value) in &self.http_headers {
            req = req.header(name, value);
        }
        req
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertex_client() -> Result<Vertex> {
        let oauth = OAuthClientCredentials::new(
            "https://auth.example/token",
            "client-id",
            "client-secret",
        )?;
        Vertex::new(oauth, "https://proxy.example/v1", "gemini-2.5-pro")
    }

    #[test]
    fn generate_url_respects_v1_join_ergonomics() -> Result<()> {
        let client = vertex_client()?;
        assert_eq!(
            client.generate_url("gemini-2.5-pro"),
            "https://proxy.example/v1/models/gemini-2.5-pro:generateContent"
        );
        Ok(())
    }

    #[cfg(feature = "cap-llm-streaming")]
    #[test]
    fn stream_url_appends_encoded_alt_and_http_query_params() -> Result<()> {
        let client = vertex_client()?.with_http_query_params(BTreeMap::from([
            ("label".to_string(), "us east".to_string()),
            ("tenant".to_string(), "team/a".to_string()),
        ]));

        assert_eq!(
            client
                .build_url_with_query_and_alt(&client.stream_url("gemini-2.5-pro"), Some("sse"))?,
            "https://proxy.example/v1/models/gemini-2.5-pro:streamGenerateContent?alt=sse&label=us+east&tenant=team%2Fa"
        );
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct VertexGenerateResponse {
    #[serde(default)]
    candidates: Vec<Value>,
    #[serde(default)]
    usage_metadata: Option<Value>,
}

#[async_trait]
impl LanguageModel for Vertex {
    fn provider(&self) -> &str {
        "vertex"
    }

    fn model_id(&self) -> &str {
        &self.default_model
    }

    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse> {
        let model = self.resolve_model(&request)?.to_string();
        let selected_provider_options =
            crate::provider_options::request_provider_options_value_for(&request, self.provider())?;
        let provider_options = selected_provider_options
            .as_ref()
            .map(crate::provider_options::ProviderOptions::from_value_ref)
            .transpose()?
            .unwrap_or_default();

        let mut warnings = Vec::<Warning>::new();
        crate::provider_options::warn_unsupported_provider_options(
            "Vertex GenAI",
            &provider_options,
            crate::provider_options::ProviderOptionsSupport::NONE,
            &mut warnings,
        );
        crate::types::warn_unsupported_generate_request_options(
            "Vertex GenAI",
            &request,
            crate::types::GenerateRequestSupport::NONE,
            &mut warnings,
        );

        let tool_names = genai::build_tool_name_map(&request.messages);
        let (contents, system_instruction) =
            genai::convert_messages(&model, &request.messages, &tool_names, &mut warnings)?;

        let mut body = Map::<String, Value>::new();
        body.insert("contents".to_string(), Value::Array(contents));

        if let Some(system_instruction) = system_instruction {
            body.insert("systemInstruction".to_string(), system_instruction);
        }

        let mut generation_config = Map::<String, Value>::new();
        if let Some(max_tokens) = request.max_tokens {
            generation_config.insert(
                "maxOutputTokens".to_string(),
                Value::Number(max_tokens.into()),
            );
        }
        if let Some(temperature) = request.temperature
            && let Some(value) = crate::utils::params::clamped_number_from_f32(
                "temperature",
                temperature,
                0.0,
                2.0,
                &mut warnings,
            )
        {
            generation_config.insert("temperature".to_string(), Value::Number(value));
        }
        if let Some(top_p) = request.top_p
            && let Some(value) = crate::utils::params::clamped_number_from_f32(
                "top_p",
                top_p,
                0.0,
                1.0,
                &mut warnings,
            )
        {
            generation_config.insert("topP".to_string(), Value::Number(value));
        }
        if let Some(stop_sequences) = request.stop_sequences {
            let stop_sequences =
                crate::utils::params::sanitize_stop_sequences(&stop_sequences, None, &mut warnings);
            if !stop_sequences.is_empty() {
                generation_config.insert(
                    "stopSequences".to_string(),
                    Value::Array(stop_sequences.into_iter().map(Value::String).collect()),
                );
            }
        }
        if !generation_config.is_empty() {
            body.insert(
                "generationConfig".to_string(),
                Value::Object(generation_config),
            );
        }

        if let Some(tools) = request.tools {
            if cfg!(feature = "cap-llm-tools") {
                let decls = tools
                    .into_iter()
                    .map(|tool| genai::tool_to_google(tool, &mut warnings))
                    .collect::<Vec<_>>();
                body.insert(
                    "tools".to_string(),
                    Value::Array(vec![serde_json::json!({ "functionDeclarations": decls })]),
                );
            } else {
                warnings.push(Warning::Unsupported {
                    feature: "tools".to_string(),
                    details: Some("ditto-core built without tools feature".to_string()),
                });
            }
        }

        if let Some(tool_choice) = request.tool_choice.as_ref()
            && cfg!(feature = "cap-llm-tools")
            && let Some(tool_config) = genai::tool_config(Some(tool_choice))
        {
            body.insert("toolConfig".to_string(), tool_config);
        }

        crate::provider_options::merge_provider_options_into_body(
            &mut body,
            selected_provider_options.as_ref(),
            &["reasoning_effort", "response_format", "parallel_tool_calls"],
            "generate.provider_options",
            &mut warnings,
        );

        let url = self.generate_url(&model);
        let url = self.build_url_with_query_and_alt(&url, None)?;
        let req = self.http.post(url).json(&body);
        let req = self.apply_headers(req);
        let req = self.apply_auth(req).await?;
        let parsed =
            crate::provider_transport::send_checked_json::<VertexGenerateResponse>(req).await?;
        let mut tool_call_seq = 0u64;
        let mut has_tool_calls = false;
        let mut content = Vec::<ContentPart>::new();

        let finish_reason_str = parsed
            .candidates
            .first()
            .and_then(|c| c.get("finishReason"))
            .and_then(Value::as_str);

        if let Some(candidate) = parsed.candidates.first() {
            content.extend(genai::parse_google_candidate(
                candidate,
                &mut tool_call_seq,
                &mut has_tool_calls,
            ));
        }

        let usage = parsed
            .usage_metadata
            .as_ref()
            .map(genai::parse_usage_metadata)
            .unwrap_or_default();

        let finish_reason = genai::map_finish_reason(finish_reason_str, has_tool_calls);

        Ok(GenerateResponse {
            content,
            finish_reason,
            usage,
            warnings,
            provider_metadata: None,
        })
    }

    async fn stream(&self, request: GenerateRequest) -> Result<StreamResult> {
        #[cfg(not(feature = "cap-llm-streaming"))]
        {
            let _ = request;
            Err(DittoError::builder_capability_feature_missing(
                "vertex",
                "streaming",
            ))
        }

        #[cfg(feature = "cap-llm-streaming")]
        {
            let model = self.resolve_model(&request)?.to_string();
            let selected_provider_options =
                crate::provider_options::request_provider_options_value_for(
                    &request,
                    self.provider(),
                )?;
            let provider_options = selected_provider_options
                .as_ref()
                .map(crate::provider_options::ProviderOptions::from_value_ref)
                .transpose()?
                .unwrap_or_default();

            let mut warnings = Vec::<Warning>::new();
            crate::provider_options::warn_unsupported_provider_options(
                "Vertex GenAI",
                &provider_options,
                crate::provider_options::ProviderOptionsSupport::NONE,
                &mut warnings,
            );
            crate::types::warn_unsupported_generate_request_options(
                "Vertex GenAI",
                &request,
                crate::types::GenerateRequestSupport::NONE,
                &mut warnings,
            );

            let tool_names = genai::build_tool_name_map(&request.messages);
            let (contents, system_instruction) =
                genai::convert_messages(&model, &request.messages, &tool_names, &mut warnings)?;

            let mut body = Map::<String, Value>::new();
            body.insert("contents".to_string(), Value::Array(contents));

            if let Some(system_instruction) = system_instruction {
                body.insert("systemInstruction".to_string(), system_instruction);
            }

            let mut generation_config = Map::<String, Value>::new();
            if let Some(max_tokens) = request.max_tokens {
                generation_config.insert(
                    "maxOutputTokens".to_string(),
                    Value::Number(max_tokens.into()),
                );
            }
            if let Some(temperature) = request.temperature
                && let Some(value) = crate::utils::params::clamped_number_from_f32(
                    "temperature",
                    temperature,
                    0.0,
                    2.0,
                    &mut warnings,
                )
            {
                generation_config.insert("temperature".to_string(), Value::Number(value));
            }
            if let Some(top_p) = request.top_p
                && let Some(value) = crate::utils::params::clamped_number_from_f32(
                    "top_p",
                    top_p,
                    0.0,
                    1.0,
                    &mut warnings,
                )
            {
                generation_config.insert("topP".to_string(), Value::Number(value));
            }
            if let Some(stop_sequences) = request.stop_sequences {
                let stop_sequences = crate::utils::params::sanitize_stop_sequences(
                    &stop_sequences,
                    None,
                    &mut warnings,
                );
                if !stop_sequences.is_empty() {
                    generation_config.insert(
                        "stopSequences".to_string(),
                        Value::Array(stop_sequences.into_iter().map(Value::String).collect()),
                    );
                }
            }
            if !generation_config.is_empty() {
                body.insert(
                    "generationConfig".to_string(),
                    Value::Object(generation_config),
                );
            }

            if let Some(tools) = request.tools
                && cfg!(feature = "cap-llm-tools")
            {
                let decls = tools
                    .into_iter()
                    .map(|tool| genai::tool_to_google(tool, &mut warnings))
                    .collect::<Vec<_>>();
                body.insert(
                    "tools".to_string(),
                    Value::Array(vec![serde_json::json!({ "functionDeclarations": decls })]),
                );
            }

            if let Some(tool_choice) = request.tool_choice.as_ref()
                && cfg!(feature = "cap-llm-tools")
                && let Some(tool_config) = genai::tool_config(Some(tool_choice))
            {
                body.insert("toolConfig".to_string(), tool_config);
            }

            crate::provider_options::merge_provider_options_into_body(
                &mut body,
                selected_provider_options.as_ref(),
                &["reasoning_effort", "response_format", "parallel_tool_calls"],
                "stream.provider_options",
                &mut warnings,
            );

            let url = self.stream_url(&model);
            let url = self.build_url_with_query_and_alt(&url, Some("sse"))?;
            let req = self.http.post(url);
            let req = self.apply_headers(req);
            let req = req.header("Accept", "text/event-stream").json(&body);
            let req = self.apply_auth(req).await?;
            let response = crate::provider_transport::send_checked(req).await?;

            let (data_stream, buffer) =
                crate::session_transport::init_sse_stream(response, warnings);

            let stream = stream::unfold(
                (
                    data_stream,
                    buffer,
                    false,
                    String::new(),
                    false,
                    None::<String>,
                    None::<Usage>,
                    0u64,
                ),
                |(
                    mut data_stream,
                    mut buffer,
                    mut done,
                    mut last_text,
                    mut has_tool_calls,
                    mut pending_finish_reason,
                    mut pending_usage,
                    mut tool_call_seq,
                )| async move {
                    loop {
                        if let Some(item) = buffer.pop_front() {
                            return Some((
                                item,
                                (
                                    data_stream,
                                    buffer,
                                    done,
                                    last_text,
                                    has_tool_calls,
                                    pending_finish_reason,
                                    pending_usage,
                                    tool_call_seq,
                                ),
                            ));
                        }

                        if done {
                            return None;
                        }

                        let next = data_stream.next().await;
                        match next {
                            Some(Ok(data)) => match serde_json::from_str::<Value>(&data) {
                                Ok(chunk) => {
                                    if let Some(usage) = chunk.get("usageMetadata") {
                                        pending_usage = Some(genai::parse_usage_metadata(usage));
                                    }
                                    if let Some(finish) = chunk
                                        .get("candidates")
                                        .and_then(Value::as_array)
                                        .and_then(|c| c.first())
                                        .and_then(|c| c.get("finishReason"))
                                        .and_then(Value::as_str)
                                    {
                                        pending_finish_reason = Some(finish.to_string());
                                    }

                                    if let Some(candidate) = chunk
                                        .get("candidates")
                                        .and_then(Value::as_array)
                                        .and_then(|c| c.first())
                                    {
                                        let parts = candidate
                                            .get("content")
                                            .and_then(|c| c.get("parts"))
                                            .and_then(Value::as_array)
                                            .cloned()
                                            .unwrap_or_default();

                                        for part in parts {
                                            if let Some(text) =
                                                part.get("text").and_then(Value::as_str)
                                            {
                                                let delta = if text.starts_with(&last_text) {
                                                    text[last_text.len()..].to_string()
                                                } else {
                                                    text.to_string()
                                                };
                                                last_text = text.to_string();
                                                if !delta.is_empty() {
                                                    buffer.push_back(Ok(StreamChunk::TextDelta {
                                                        text: delta,
                                                    }));
                                                }
                                                continue;
                                            }
                                            if let Some(call) = part.get("functionCall") {
                                                let Some(name) =
                                                    call.get("name").and_then(Value::as_str)
                                                else {
                                                    continue;
                                                };
                                                let args = call
                                                    .get("args")
                                                    .cloned()
                                                    .unwrap_or(Value::Null);
                                                let id = format!("call_{}", tool_call_seq);
                                                tool_call_seq = tool_call_seq.saturating_add(1);
                                                has_tool_calls = true;
                                                buffer.push_back(Ok(StreamChunk::ToolCallStart {
                                                    id: id.clone(),
                                                    name: name.to_string(),
                                                }));
                                                buffer.push_back(Ok(StreamChunk::ToolCallDelta {
                                                    id,
                                                    arguments_delta: args.to_string(),
                                                }));
                                            }
                                        }
                                    }
                                }
                                Err(err) => {
                                    done = true;
                                    buffer.push_back(Err(err.into()));
                                }
                            },
                            Some(Err(err)) => {
                                done = true;
                                buffer.push_back(Err(err));
                            }
                            None => {
                                done = true;
                                if let Some(usage) = pending_usage.take() {
                                    buffer.push_back(Ok(StreamChunk::Usage(usage)));
                                }
                                buffer.push_back(Ok(StreamChunk::FinishReason(
                                    genai::map_finish_reason(
                                        pending_finish_reason.as_deref(),
                                        has_tool_calls,
                                    ),
                                )));
                            }
                        }
                    }
                },
            );

            Ok(Box::pin(stream))
        }
    }
}
