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

#[derive(Debug, Deserialize)]
struct GoogleGenerateResponse {
    #[serde(default)]
    candidates: Vec<Value>,
    #[serde(default)]
    usage_metadata: Option<Value>,
}

fn parse_google_candidate(
    candidate: &Value,
    tool_call_seq: &mut u64,
    has_tool_calls: &mut bool,
) -> Vec<ContentPart> {
    genai::parse_google_candidate(candidate, tool_call_seq, has_tool_calls)
}

#[async_trait]
impl LanguageModel for Google {
    fn provider(&self) -> &str {
        "google"
    }

    fn model_id(&self) -> &str {
        self.default_model.as_str()
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
        if provider_options.reasoning_effort.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "reasoning_effort".to_string(),
                details: Some("Google GenAI does not support reasoning_effort".to_string()),
            });
        }
        if provider_options.response_format.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "response_format".to_string(),
                details: Some("Google GenAI does not support response_format".to_string()),
            });
        }
        if provider_options.parallel_tool_calls == Some(true) {
            warnings.push(Warning::Unsupported {
                feature: "parallel_tool_calls".to_string(),
                details: Some("Google GenAI does not support parallel_tool_calls".to_string()),
            });
        }
        let tool_names = Self::build_tool_name_map(&request.messages);
        let (contents, system_instruction) =
            Self::convert_messages(&model, &request.messages, &tool_names, &mut warnings)?;

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
                    .map(|tool| Self::tool_to_google(tool, &mut warnings))
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
                if let Some(tool_config) = Self::tool_config(Some(tool_choice)) {
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
        let req = self.http.post(url);
        let response = self.apply_auth(req).json(&body).send().await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let parsed = response.json::<GoogleGenerateResponse>().await?;
        let mut tool_call_seq = 0u64;
        let mut has_tool_calls = false;
        let mut content = Vec::<ContentPart>::new();

        let finish_reason_str = parsed
            .candidates
            .first()
            .and_then(|c| c.get("finishReason"))
            .and_then(Value::as_str);

        if let Some(candidate) = parsed.candidates.first() {
            content.extend(parse_google_candidate(
                candidate,
                &mut tool_call_seq,
                &mut has_tool_calls,
            ));
        }

        let usage = parsed
            .usage_metadata
            .as_ref()
            .map(Self::parse_usage_metadata)
            .unwrap_or_default();

        let finish_reason = Self::map_finish_reason(finish_reason_str, has_tool_calls);

        Ok(GenerateResponse {
            content,
            finish_reason,
            usage,
            warnings,
            provider_metadata: None,
        })
    }

    async fn stream(&self, request: GenerateRequest) -> Result<StreamResult> {
        #[cfg(not(feature = "streaming"))]
        {
            let _ = request;
            return Err(DittoError::InvalidResponse(
                "ditto-llm built without streaming feature".to_string(),
            ));
        }

        #[cfg(feature = "streaming")]
        {
            let model = self.resolve_model(&request)?.to_string();
            let selected_provider_options = request.provider_options_value_for(self.provider())?;
            let provider_options = selected_provider_options
                .as_ref()
                .map(crate::types::ProviderOptions::from_value)
                .transpose()?
                .unwrap_or_default();

            let mut warnings = Vec::<Warning>::new();
            if provider_options.reasoning_effort.is_some() {
                warnings.push(Warning::Unsupported {
                    feature: "reasoning_effort".to_string(),
                    details: Some("Google GenAI does not support reasoning_effort".to_string()),
                });
            }
            if provider_options.response_format.is_some() {
                warnings.push(Warning::Unsupported {
                    feature: "response_format".to_string(),
                    details: Some("Google GenAI does not support response_format".to_string()),
                });
            }
            if provider_options.parallel_tool_calls == Some(true) {
                warnings.push(Warning::Unsupported {
                    feature: "parallel_tool_calls".to_string(),
                    details: Some("Google GenAI does not support parallel_tool_calls".to_string()),
                });
            }
            let tool_names = Self::build_tool_name_map(&request.messages);
            let (contents, system_instruction) =
                Self::convert_messages(&model, &request.messages, &tool_names, &mut warnings)?;

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
                        .map(|tool| Self::tool_to_google(tool, &mut warnings))
                        .collect::<Vec<_>>();
                    body.insert(
                        "tools".to_string(),
                        Value::Array(vec![serde_json::json!({ "functionDeclarations": decls })]),
                    );
                }
            }

            if let Some(tool_choice) = request.tool_choice.as_ref() {
                if cfg!(feature = "tools") {
                    if let Some(tool_config) = Self::tool_config(Some(tool_choice)) {
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
            let req = self.http.post(url);
            let response = self
                .apply_auth(req)
                .header("Accept", "text/event-stream")
                .json(&body)
                .send()
                .await?;

            let status = response.status();
            if !status.is_success() {
                let text = response.text().await.unwrap_or_default();
                return Err(DittoError::Api { status, body: text });
            }

            let data_stream = crate::utils::sse::sse_data_stream_from_response(response);
            let mut buffer = VecDeque::<Result<StreamChunk>>::new();
            if !warnings.is_empty() {
                buffer.push_back(Ok(StreamChunk::Warnings { warnings }));
            }

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
                                        pending_usage = Some(Self::parse_usage_metadata(usage));
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
                                    Self::map_finish_reason(
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

#[cfg(feature = "embeddings")]
#[derive(Clone)]
pub struct GoogleEmbeddings {
    http: reqwest::Client,
    base_url: String,
    auth: Option<RequestAuth>,
    model: String,
    http_query_params: BTreeMap<String, String>,
}

#[cfg(feature = "embeddings")]
impl GoogleEmbeddings {
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
            model: String::new(),
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
        self.model = model.into();
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

    fn resolve_model(&self) -> Result<&str> {
        if !self.model.trim().is_empty() {
            return Ok(self.model.as_str());
        }
        Err(DittoError::InvalidResponse(
            "google embedding model is not set (set GoogleEmbeddings::with_model)".to_string(),
        ))
    }

    fn embed_url(&self, suffix: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        let model = Google::model_path(self.model.as_str());
        format!("{base}/{model}:{suffix}")
    }
}

#[cfg(feature = "embeddings")]
#[derive(Debug, Deserialize)]
struct BatchEmbedResponse {
    #[serde(default)]
    embeddings: Vec<EmbeddingItem>,
}

#[cfg(feature = "embeddings")]
#[derive(Debug, Deserialize)]
struct SingleEmbedResponse {
    embedding: EmbeddingItem,
}

#[cfg(feature = "embeddings")]
#[derive(Debug, Deserialize)]
struct EmbeddingItem {
    values: Vec<f32>,
}

#[cfg(feature = "embeddings")]
#[async_trait]
impl EmbeddingModel for GoogleEmbeddings {
    fn provider(&self) -> &str {
        "google"
    }

    fn model_id(&self) -> &str {
        self.model.as_str()
    }

    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let model = self.resolve_model()?;
        let _ = model;

        if texts.len() == 1 {
            let url = self.embed_url("embedContent");
            let req = self.http.post(url);
            let response = self
                .apply_auth(req)
                .json(&serde_json::json!({
                    "model": Google::model_path(self.model.as_str()),
                    "content": { "parts": [{ "text": texts[0] }] }
                }))
                .send()
                .await?;

            let status = response.status();
            if !status.is_success() {
                let text = response.text().await.unwrap_or_default();
                return Err(DittoError::Api { status, body: text });
            }

            let parsed = response.json::<SingleEmbedResponse>().await?;
            return Ok(vec![parsed.embedding.values]);
        }

        let url = self.embed_url("batchEmbedContents");
        let requests = texts
            .into_iter()
            .map(|text| {
                serde_json::json!({
                    "model": Google::model_path(self.model.as_str()),
                    "content": { "role": "user", "parts": [{ "text": text }] }
                })
            })
            .collect::<Vec<_>>();

        let req = self.http.post(url);
        let response = self
            .apply_auth(req)
            .json(&serde_json::json!({ "requests": requests }))
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DittoError::Api { status, body: text });
        }

        let parsed = response.json::<BatchEmbedResponse>().await?;
        Ok(parsed.embeddings.into_iter().map(|e| e.values).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FileSource, Role};
    use serde_json::json;

    #[test]
    fn converts_system_to_system_instruction() -> crate::Result<()> {
        let mut warnings = Vec::new();
        let tool_names = HashMap::new();
        let (contents, system) = Google::convert_messages(
            "gemini-pro",
            &[Message::system("sys"), Message::user("hi")],
            &tool_names,
            &mut warnings,
        )?;
        assert_eq!(warnings.len(), 0);
        assert_eq!(contents.len(), 1);
        assert!(system.is_some());
        Ok(())
    }

    #[test]
    fn tool_choice_maps_to_tool_config() {
        let config = Google::tool_config(Some(&ToolChoice::Tool {
            name: "add".to_string(),
        }))
        .expect("tool config");
        assert_eq!(
            config
                .get("functionCallingConfig")
                .and_then(|v| v.get("mode"))
                .and_then(Value::as_str),
            Some("ANY")
        );
    }

    #[test]
    fn tool_declaration_converts_schema() {
        let tool = Tool {
            name: "add".to_string(),
            description: Some("add".to_string()),
            parameters: json!({
                "type": "object",
                "properties": { "a": { "type": "integer" } }
            }),
            strict: None,
        };
        let mut warnings = Vec::new();
        let decl = Google::tool_to_google(tool, &mut warnings);
        assert!(warnings.is_empty());
        assert_eq!(decl.get("name").and_then(Value::as_str), Some("add"));
        assert!(decl.get("parameters").is_some());
    }

    #[test]
    fn tool_schema_ref_is_resolved_without_warning() {
        let tool = Tool {
            name: "add".to_string(),
            description: Some("add".to_string()),
            parameters: json!({
                "$ref": "#/$defs/Args",
                "$defs": {
                    "Args": { "type": "object", "properties": { "a": { "type": "integer" } } }
                }
            }),
            strict: None,
        };
        let mut warnings = Vec::new();
        let decl = Google::tool_to_google(tool, &mut warnings);
        assert_eq!(decl.get("name").and_then(Value::as_str), Some("add"));
        assert!(warnings.is_empty());
        assert_eq!(
            decl.get("parameters"),
            Some(&json!({
                "type": "object",
                "properties": { "a": { "type": "integer" } }
            }))
        );
    }

    #[test]
    fn tool_schema_unresolvable_ref_emits_warning() {
        let tool = Tool {
            name: "add".to_string(),
            description: Some("add".to_string()),
            parameters: json!({
                "$ref": "#/$defs/Missing",
                "$defs": {
                    "Args": { "type": "object", "properties": { "a": { "type": "integer" } } }
                }
            }),
            strict: None,
        };
        let mut warnings = Vec::new();
        let decl = Google::tool_to_google(tool, &mut warnings);
        assert_eq!(decl.get("name").and_then(Value::as_str), Some("add"));
        assert_eq!(decl.get("parameters"), Some(&json!({})));
        assert!(warnings.iter().any(|w| {
            matches!(w, Warning::Compatibility { feature, .. } if feature == "tool.parameters.$ref")
        }));
    }

    #[test]
    fn tool_schema_unsupported_keywords_emit_warning() {
        let tool = Tool {
            name: "add".to_string(),
            description: Some("add".to_string()),
            parameters: json!({
                "type": "object",
                "properties": { "a": { "type": "integer" } },
                "not": { "type": "object" }
            }),
            strict: None,
        };
        let mut warnings = Vec::new();
        let decl = Google::tool_to_google(tool, &mut warnings);
        assert_eq!(decl.get("name").and_then(Value::as_str), Some("add"));
        assert!(warnings.iter().any(|w| matches!(
            w,
            Warning::Compatibility { feature, details } if feature == "tool.parameters.unsupported_keywords" && details.contains("not")
        )));
    }

    #[test]
    fn converts_pdf_file_part_to_inline_data() -> crate::Result<()> {
        let mut warnings = Vec::new();
        let tool_names = HashMap::new();
        let (contents, _system) = Google::convert_messages(
            "gemini-pro",
            &[Message {
                role: Role::User,
                content: vec![ContentPart::File {
                    filename: Some("doc.pdf".to_string()),
                    media_type: "application/pdf".to_string(),
                    source: FileSource::Base64 {
                        data: "AQIDBAU=".to_string(),
                    },
                }],
            }],
            &tool_names,
            &mut warnings,
        )?;
        assert!(warnings.is_empty());
        assert_eq!(contents.len(), 1);
        let parts = contents[0]
            .get("parts")
            .and_then(Value::as_array)
            .expect("parts array");
        assert_eq!(parts.len(), 1);
        assert_eq!(
            parts[0]
                .get("inlineData")
                .and_then(Value::as_object)
                .and_then(|o| o.get("mimeType"))
                .and_then(Value::as_str),
            Some("application/pdf")
        );
        Ok(())
    }
}
