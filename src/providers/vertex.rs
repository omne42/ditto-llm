use std::collections::BTreeMap;

use async_trait::async_trait;
use futures_util::StreamExt;
use futures_util::stream;
use reqwest::Url;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::genai;
use crate::auth::oauth::{OAuthClientCredentials, resolve_oauth_client_credentials};
use crate::model::{LanguageModel, StreamResult};
use crate::profile::{Env, HttpAuth, ProviderConfig};
use crate::types::{ContentPart, GenerateRequest, GenerateResponse, StreamChunk, Usage, Warning};
use crate::{DittoError, Result};

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
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .map_err(DittoError::Http)?;
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
        let base_url = config.base_url.as_deref().ok_or_else(|| {
            DittoError::InvalidResponse("provider base_url is missing".to_string())
        })?;
        let model = config.default_model.as_deref().ok_or_else(|| {
            DittoError::InvalidResponse("provider default_model is missing".to_string())
        })?;
        let auth = config
            .auth
            .clone()
            .ok_or_else(|| DittoError::InvalidResponse("vertex auth is missing".to_string()))?;
        let oauth = resolve_oauth_client_credentials(&auth, env)?;

        let mut out = Self::new(oauth, base_url, model)?;
        out.http_headers = config.http_headers.clone();
        out.http_query_params = config.http_query_params.clone();
        Ok(out)
    }

    fn resolve_model<'a>(&'a self, request: &'a GenerateRequest) -> Result<&'a str> {
        if let Some(model) = request.model.as_deref().filter(|m| !m.trim().is_empty()) {
            return Ok(model);
        }
        if !self.default_model.trim().is_empty() {
            return Ok(self.default_model.as_str());
        }
        Err(DittoError::InvalidResponse(
            "vertex model is not set".to_string(),
        ))
    }

    fn generate_url(&self, model: &str) -> String {
        if self.base_url.contains("{model}") {
            return self.base_url.replace("{model}", model);
        }
        if self.base_url.ends_with(":generateContent") {
            return self.base_url.clone();
        }
        let base = self.base_url.trim_end_matches('/');
        format!("{base}/models/{model}:generateContent")
    }

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
        let base = self.base_url.trim_end_matches('/');
        format!("{base}/models/{model}:streamGenerateContent")
    }

    fn build_url_with_query_and_alt(&self, base: &str, alt: Option<&str>) -> Result<String> {
        let mut url = Url::parse(base).map_err(|err| {
            DittoError::InvalidResponse(format!("invalid vertex base_url {base:?}: {err}"))
        })?;
        {
            let mut pairs = url.query_pairs_mut();
            if let Some(alt) = alt {
                pairs.append_pair("alt", alt);
            }
            for (key, value) in &self.http_query_params {
                if key.trim().is_empty() {
                    continue;
                }
                pairs.append_pair(key, value);
            }
        }
        Ok(url.to_string())
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
        let selected_provider_options = request.provider_options_value_for(self.provider())?;
        let provider_options = selected_provider_options
            .as_ref()
            .map(crate::types::ProviderOptions::from_value)
            .transpose()?
            .unwrap_or_default();

        let mut warnings = Vec::<Warning>::new();
        crate::types::warn_unsupported_provider_options(
            "Vertex GenAI",
            &provider_options,
            crate::types::ProviderOptionsSupport::NONE,
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
        if let Some(temperature) = request.temperature {
            if let Some(value) = crate::utils::params::clamped_number_from_f32(
                "temperature",
                temperature,
                0.0,
                2.0,
                &mut warnings,
            ) {
                generation_config.insert("temperature".to_string(), Value::Number(value));
            }
        }
        if let Some(top_p) = request.top_p {
            if let Some(value) = crate::utils::params::clamped_number_from_f32(
                "top_p",
                top_p,
                0.0,
                1.0,
                &mut warnings,
            ) {
                generation_config.insert("topP".to_string(), Value::Number(value));
            }
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
            if cfg!(feature = "tools") {
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
                    details: Some("ditto-llm built without tools feature".to_string()),
                });
            }
        }

        if let Some(tool_choice) = request.tool_choice.as_ref() {
            if cfg!(feature = "tools") {
                if let Some(tool_config) = genai::tool_config(Some(tool_choice)) {
                    body.insert("toolConfig".to_string(), tool_config);
                }
            }
        }

        crate::types::merge_provider_options_into_body(
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
        let parsed = crate::utils::http::send_checked_json::<VertexGenerateResponse>(req).await?;
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

    async fn stream(&self, _request: GenerateRequest) -> Result<StreamResult> {
        #[cfg(not(feature = "streaming"))]
        {
            let _ = _request;
            return Err(DittoError::InvalidResponse(
                "ditto-llm built without streaming feature".to_string(),
            ));
        }

        #[cfg(feature = "streaming")]
        {
            let request = _request;
            let model = self.resolve_model(&request)?.to_string();
            let selected_provider_options = request.provider_options_value_for(self.provider())?;
            let provider_options = selected_provider_options
                .as_ref()
                .map(crate::types::ProviderOptions::from_value)
                .transpose()?
                .unwrap_or_default();

            let mut warnings = Vec::<Warning>::new();
            crate::types::warn_unsupported_provider_options(
                "Vertex GenAI",
                &provider_options,
                crate::types::ProviderOptionsSupport::NONE,
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
            if let Some(temperature) = request.temperature {
                if let Some(value) = crate::utils::params::clamped_number_from_f32(
                    "temperature",
                    temperature,
                    0.0,
                    2.0,
                    &mut warnings,
                ) {
                    generation_config.insert("temperature".to_string(), Value::Number(value));
                }
            }
            if let Some(top_p) = request.top_p {
                if let Some(value) = crate::utils::params::clamped_number_from_f32(
                    "top_p",
                    top_p,
                    0.0,
                    1.0,
                    &mut warnings,
                ) {
                    generation_config.insert("topP".to_string(), Value::Number(value));
                }
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

            if let Some(tools) = request.tools {
                if cfg!(feature = "tools") {
                    let decls = tools
                        .into_iter()
                        .map(|tool| genai::tool_to_google(tool, &mut warnings))
                        .collect::<Vec<_>>();
                    body.insert(
                        "tools".to_string(),
                        Value::Array(vec![serde_json::json!({ "functionDeclarations": decls })]),
                    );
                }
            }

            if let Some(tool_choice) = request.tool_choice.as_ref() {
                if cfg!(feature = "tools") {
                    if let Some(tool_config) = genai::tool_config(Some(tool_choice)) {
                        body.insert("toolConfig".to_string(), tool_config);
                    }
                }
            }

            crate::types::merge_provider_options_into_body(
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
            let response = crate::utils::http::send_checked(req).await?;

            let (data_stream, buffer) =
                crate::utils::streaming::init_sse_stream(response, warnings);

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
