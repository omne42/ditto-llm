use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Map, Value};

use super::openai_like::OpenAiLikeClient;

use crate::Result;
use crate::types::{ModerationRequest, ModerationResponse, ModerationResult, Warning};

#[derive(Debug, Deserialize)]
struct ModerationsResponse {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    results: Vec<ModerationsResult>,
}

#[derive(Debug, Deserialize)]
struct ModerationsResult {
    #[serde(default)]
    flagged: bool,
    #[serde(default)]
    categories: BTreeMap<String, bool>,
    #[serde(default)]
    category_scores: BTreeMap<String, f64>,
}

pub(super) async fn moderate(
    provider: &str,
    client: &OpenAiLikeClient,
    model: String,
    request: ModerationRequest,
) -> Result<ModerationResponse> {
    let ModerationRequest {
        input,
        model: _,
        provider_options,
    } = request;

    let selected_provider_options =
        crate::types::select_provider_options_value(provider_options.as_ref(), provider)?;
    let mut warnings = Vec::<Warning>::new();

    let mut body = Map::<String, Value>::new();
    body.insert("model".to_string(), Value::String(model.clone()));
    body.insert("input".to_string(), serde_json::to_value(&input)?);

    crate::types::merge_provider_options_into_body(
        &mut body,
        selected_provider_options.as_ref(),
        &["model", "input"],
        "moderation.provider_options",
        &mut warnings,
    );

    let url = client.endpoint("moderations");
    let parsed = crate::utils::http::send_checked_json::<ModerationsResponse>(
        client.apply_auth(client.http.post(url)).json(&body),
    )
    .await?;

    let results = parsed
        .results
        .into_iter()
        .map(|result| ModerationResult {
            flagged: result.flagged,
            categories: result.categories,
            category_scores: result.category_scores,
            provider_metadata: None,
        })
        .collect();

    Ok(ModerationResponse {
        id: parsed.id,
        model: parsed.model.or_else(|| Some(model.clone())),
        results,
        warnings,
        provider_metadata: Some(serde_json::json!({ "model": model })),
    })
}
