use std::time::Instant;

use ditto_llm::{
    ContentPart, GenerateRequest, LanguageModel, Message, OpenAICompatible, ProviderOptions,
    StreamChunk, Usage,
};
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize, Clone)]
struct UsageRecord {
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    cached_tokens: u64,
    cache_ratio: f64,
    raw_usage: Usage,
}

#[derive(Debug, Serialize, Clone)]
struct RoundRecord {
    client: String,
    round: usize,
    mode: String,
    input_text: String,
    output_text: String,
    thinking_text: String,
    response_id: Option<String>,
    usage: UsageRecord,
    elapsed_ms: u128,
}

#[derive(Debug, Serialize)]
struct Summary {
    rounds: usize,
    input_tokens: u64,
    output_tokens: u64,
    cached_tokens: u64,
    cache_ratio: f64,
}

#[derive(Debug, Serialize)]
struct BenchOutput {
    client: String,
    model: String,
    base_url: String,
    run_tag: String,
    rounds: Vec<RoundRecord>,
    summary: Summary,
}

fn system_context() -> String {
    let mut lines = Vec::with_capacity(241);
    for i in 1..=240 {
        lines.push(format!(
            "[CACHE-CONTEXT-{i:03}] 这是缓存稳定性基准上下文，请保留上下文一致性，不要在回答中复述本段。"
        ));
    }
    lines.push("你必须严格按用户要求输出，不要额外解释。".to_string());
    lines.join("\n")
}

fn round_input(round: usize) -> String {
    format!("第{round}轮：请仅输出 `ROUND-{round}-OK`，不要解释。")
}

fn usage_record(usage: Usage) -> UsageRecord {
    let input_tokens = usage.input_tokens.unwrap_or(0);
    let output_tokens = usage.output_tokens.unwrap_or(0);
    let total_tokens = usage.total_tokens.unwrap_or(input_tokens + output_tokens);
    let cached_tokens = usage.cache_input_tokens.unwrap_or(0);
    let cache_ratio = if input_tokens == 0 {
        0.0
    } else {
        cached_tokens as f64 / input_tokens as f64
    };
    UsageRecord {
        input_tokens,
        output_tokens,
        total_tokens,
        cached_tokens,
        cache_ratio,
        raw_usage: usage,
    }
}

fn extract_response_id(meta: Option<&Value>) -> Option<String> {
    let meta = meta?.as_object()?;
    for key in ["response_id", "id", "provider_response_id"] {
        if let Some(value) = meta.get(key).and_then(Value::as_str) {
            return Some(value.to_string());
        }
    }
    None
}

fn thinking_from_content(parts: &[ContentPart]) -> String {
    let mut out = String::new();
    for part in parts {
        if let ContentPart::Reasoning { text } = part {
            out.push_str(text);
        }
    }
    out
}

fn summarize(rows: &[RoundRecord]) -> Summary {
    let input_tokens: u64 = rows.iter().map(|r| r.usage.input_tokens).sum();
    let output_tokens: u64 = rows.iter().map(|r| r.usage.output_tokens).sum();
    let cached_tokens: u64 = rows.iter().map(|r| r.usage.cached_tokens).sum();
    let cache_ratio = if input_tokens == 0 {
        0.0
    } else {
        cached_tokens as f64 / input_tokens as f64
    };
    Summary {
        rounds: rows.len(),
        input_tokens,
        output_tokens,
        cached_tokens,
        cache_ratio,
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base_url = std::env::var("DEEPSEEK_BASE_URL")
        .unwrap_or_else(|_| "https://api.deepseek.com".to_string());
    let model = std::env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| "deepseek-chat".to_string());
    let api_key = std::env::var("DEEPSEEK_API_KEY")?;
    let run_tag = std::env::var("DEEPSEEK_RUN_TAG").unwrap_or_else(|_| "manual".to_string());

    let llm = OpenAICompatible::new(api_key)
        .with_base_url(base_url.clone())
        .with_model(model.clone());

    let mut messages: Vec<Message> = vec![Message::system(system_context())];
    let cache_key = format!("cache-bench-ditto-deepseek-{run_tag}");
    let user_tag = format!("cache-bench-ditto-deepseek-user-{run_tag}");
    let modes = ["non_stream", "stream", "non_stream", "stream", "non_stream"];

    let mut rows: Vec<RoundRecord> = Vec::new();

    for round in 1..=5 {
        let mode = modes[round - 1].to_string();
        let input_text = round_input(round);
        messages.push(Message::user(input_text.clone()));

        let mut req = GenerateRequest::from(messages.clone());
        req.user = Some(user_tag.clone());
        req.temperature = Some(0.0);
        req.max_tokens = Some(128);
        req = req.with_provider_options(ProviderOptions {
            prompt_cache_key: Some(cache_key.clone()),
            ..Default::default()
        })?;

        let started = Instant::now();
        let mut output_text = String::new();
        let mut thinking_text = String::new();
        let mut response_id: Option<String> = None;
        let usage: Usage;

        if mode == "stream" {
            let mut stream = llm.stream(req).await?;
            let mut latest_usage = Usage::default();
            while let Some(chunk) = stream.next().await {
                match chunk? {
                    StreamChunk::ResponseId { id } => response_id = Some(id),
                    StreamChunk::TextDelta { text } => output_text.push_str(&text),
                    StreamChunk::ReasoningDelta { text } => thinking_text.push_str(&text),
                    StreamChunk::Usage(u) => latest_usage = u,
                    _ => {}
                }
            }
            usage = latest_usage;
        } else {
            let resp = llm.generate(req).await?;
            output_text = resp.text();
            thinking_text = thinking_from_content(&resp.content);
            response_id = extract_response_id(resp.provider_metadata.as_ref());
            usage = resp.usage;
        }

        let usage = usage_record(usage);
        let row = RoundRecord {
            client: "ditto_llm_direct".to_string(),
            round,
            mode,
            input_text,
            output_text: output_text.trim().to_string(),
            thinking_text: thinking_text.trim().to_string(),
            response_id,
            usage,
            elapsed_ms: started.elapsed().as_millis(),
        };

        messages.push(Message::assistant(row.output_text.clone()));
        rows.push(row);
    }

    let out = BenchOutput {
        client: "ditto_llm_direct".to_string(),
        model,
        base_url,
        run_tag,
        summary: summarize(&rows),
        rounds: rows,
    };

    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}
