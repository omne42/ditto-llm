use serde::Deserialize;

use crate::Result;

use super::OpenAI;

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAIModelPermission {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub object: Option<String>,
    #[serde(default)]
    pub created: Option<u64>,
    #[serde(default)]
    pub allow_create_engine: Option<bool>,
    #[serde(default)]
    pub allow_sampling: Option<bool>,
    #[serde(default)]
    pub allow_logprobs: Option<bool>,
    #[serde(default)]
    pub allow_search_indices: Option<bool>,
    #[serde(default)]
    pub allow_view: Option<bool>,
    #[serde(default)]
    pub allow_fine_tuning: Option<bool>,
    #[serde(default)]
    pub organization: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub is_blocking: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAIModelObject {
    pub id: String,
    #[serde(default)]
    pub object: Option<String>,
    #[serde(default)]
    pub created: Option<u64>,
    #[serde(default)]
    pub owned_by: Option<String>,
    #[serde(default)]
    pub permission: Vec<OpenAIModelPermission>,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub parent: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIModelsListResponse {
    #[serde(default)]
    data: Vec<OpenAIModelObject>,
}

impl OpenAI {
    pub async fn list_models(&self) -> Result<Vec<OpenAIModelObject>> {
        let url = self.client.endpoint("models");
        let req = self.client.http.get(url);
        let parsed =
            crate::utils::http::send_checked_json::<OpenAIModelsListResponse>(self.apply_auth(req))
                .await?;
        Ok(parsed.data)
    }

    pub async fn list_model_ids(&self) -> Result<Vec<String>> {
        let mut ids = self
            .list_models()
            .await?
            .into_iter()
            .map(|model| model.id)
            .collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    pub async fn retrieve_model(&self, model_id: &str) -> Result<OpenAIModelObject> {
        let url = self.client.endpoint(&format!("models/{}", model_id.trim()));
        let req = self.client.http.get(url);
        crate::utils::http::send_checked_json::<OpenAIModelObject>(self.apply_auth(req)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::{Method::GET, MockServer};

    #[tokio::test]
    async fn list_models_hits_models_endpoint() -> crate::Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v1/models")
                    .header("authorization", "Bearer sk-test");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        serde_json::json!({
                            "data": [
                                {
                                    "id": "gpt-5",
                                    "object": "model",
                                    "owned_by": "openai"
                                },
                                {
                                    "id": "gpt-4.1",
                                    "object": "model",
                                    "owned_by": "openai"
                                }
                            ]
                        })
                        .to_string(),
                    );
            })
            .await;

        let client = OpenAI::new("sk-test").with_base_url(server.url("/v1"));
        let models = client.list_models().await?;

        mock.assert_async().await;
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-5");
        assert_eq!(models[1].owned_by.as_deref(), Some("openai"));
        Ok(())
    }

    #[tokio::test]
    async fn retrieve_model_hits_model_resource_endpoint() -> crate::Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v1/models/gpt-5")
                    .header("authorization", "Bearer sk-test");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        serde_json::json!({
                            "id": "gpt-5",
                            "object": "model",
                            "owned_by": "openai"
                        })
                        .to_string(),
                    );
            })
            .await;

        let client = OpenAI::new("sk-test").with_base_url(server.url("/v1"));
        let model = client.retrieve_model("gpt-5").await?;

        mock.assert_async().await;
        assert_eq!(model.id, "gpt-5");
        assert_eq!(model.owned_by.as_deref(), Some("openai"));
        Ok(())
    }

    #[tokio::test]
    async fn list_model_ids_sorts_and_deduplicates() -> crate::Result<()> {
        if crate::utils::test_support::should_skip_httpmock() {
            return Ok(());
        }
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/v1/models")
                    .header("authorization", "Bearer sk-test");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        serde_json::json!({
                            "data": [
                                { "id": "gpt-4.1", "object": "model" },
                                { "id": "gpt-5", "object": "model" },
                                { "id": "gpt-4.1", "object": "model" }
                            ]
                        })
                        .to_string(),
                    );
            })
            .await;

        let client = OpenAI::new("sk-test").with_base_url(server.url("/v1"));
        let ids = client.list_model_ids().await?;

        mock.assert_async().await;
        assert_eq!(ids, vec!["gpt-4.1".to_string(), "gpt-5".to_string()]);
        Ok(())
    }
}
