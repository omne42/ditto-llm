use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{DittoError, Result};

#[cfg(any(
    feature = "anthropic",
    feature = "bedrock",
    feature = "cohere",
    feature = "google",
    feature = "openai",
    feature = "openai-compatible",
    feature = "vertex",
))]
use super::Warning;
use super::{ProviderOptions, ResponseFormat};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(transparent)]
pub struct ProviderOptionsEnvelope(Value);

impl ProviderOptionsEnvelope {
    pub fn from_options(options: ProviderOptions) -> Result<Self> {
        Ok(Self(serde_json::to_value(options)?))
    }

    pub fn as_value(&self) -> &Value {
        &self.0
    }

    pub fn into_value(self) -> Value {
        self.0
    }

    pub fn provider_options_for(&self, provider: &str) -> Option<&Value> {
        select_provider_options(Some(self), provider)
    }

    pub fn provider_options_value_for(&self, provider: &str) -> Result<Option<Value>> {
        select_provider_options_value(Some(self), provider)
    }

    pub fn parsed_provider_options_for(&self, provider: &str) -> Result<Option<ProviderOptions>> {
        let selected = self.provider_options_value_for(provider)?;
        selected
            .as_ref()
            .map(ProviderOptions::from_value)
            .transpose()
    }

    pub fn parsed_provider_options(&self) -> Result<Option<ProviderOptions>> {
        if self
            .0
            .as_object()
            .map(provider_options_object_is_bucketed)
            .unwrap_or(false)
        {
            return Ok(None);
        }

        ProviderOptions::from_value(&self.0).map(Some)
    }

    pub fn merge_response_format_for_provider(
        provider_options: Option<Self>,
        provider: &str,
        response_format: ResponseFormat,
    ) -> Result<Self> {
        let response_format_value = serde_json::to_value(response_format)?;
        let merged = match provider_options.map(Self::into_value) {
            None => {
                let mut obj = Map::<String, Value>::new();
                obj.insert("response_format".to_string(), response_format_value);
                Value::Object(obj)
            }
            Some(Value::Object(mut obj)) => {
                if provider_options_object_is_bucketed(&obj) {
                    let slot = obj
                        .entry(provider.to_string())
                        .or_insert_with(|| Value::Object(Map::new()));
                    let Value::Object(bucket) = slot else {
                        return Err(DittoError::InvalidResponse(format!(
                            "invalid provider_options: bucket {provider:?} must be a JSON object"
                        )));
                    };
                    bucket.insert("response_format".to_string(), response_format_value);
                    Value::Object(obj)
                } else {
                    obj.insert("response_format".to_string(), response_format_value);
                    Value::Object(obj)
                }
            }
            Some(_) => {
                return Err(DittoError::InvalidResponse(
                    "provider_options must be a JSON object".to_string(),
                ));
            }
        };
        Ok(Self(merged))
    }
}

impl AsRef<Value> for ProviderOptionsEnvelope {
    fn as_ref(&self) -> &Value {
        &self.0
    }
}

impl From<Value> for ProviderOptionsEnvelope {
    fn from(value: Value) -> Self {
        Self(value)
    }
}

pub(crate) trait ProviderOptionsSource {
    fn provider_options_value(&self) -> &Value;
}

impl ProviderOptionsSource for Value {
    fn provider_options_value(&self) -> &Value {
        self
    }
}

impl ProviderOptionsSource for ProviderOptionsEnvelope {
    fn provider_options_value(&self) -> &Value {
        self.as_value()
    }
}

const PROVIDER_OPTIONS_BUCKETS: &[&str] = &[
    "*",
    "openai",
    "openai-compatible",
    "openai_compatible",
    "anthropic",
    "google",
    "cohere",
    "bedrock",
    "vertex",
];

pub(crate) fn provider_options_object_is_bucketed(obj: &Map<String, Value>) -> bool {
    obj.keys()
        .any(|key| PROVIDER_OPTIONS_BUCKETS.contains(&key.as_str()))
}

pub(crate) fn select_provider_options<'a, T>(
    provider_options: Option<&'a T>,
    provider: &str,
) -> Option<&'a Value>
where
    T: ProviderOptionsSource + ?Sized,
{
    let provider_options = provider_options?.provider_options_value();
    let Some(obj) = provider_options.as_object() else {
        return Some(provider_options);
    };

    if provider_options_object_is_bucketed(obj) {
        if let Some(bucket) = obj.get(provider) {
            return Some(bucket);
        }
        if let Some(bucket) = provider_bucket_alias_key(provider).and_then(|key| obj.get(key)) {
            return Some(bucket);
        }
        if let Some(bucket) = obj.get("*") {
            return Some(bucket);
        }
        return None;
    }

    Some(provider_options)
}

fn provider_bucket_alias_key(provider: &str) -> Option<&str> {
    match provider {
        "openai-compatible" => Some("openai_compatible"),
        "openai_compatible" => Some("openai-compatible"),
        _ => None,
    }
}

pub(crate) fn select_provider_options_value<T>(
    provider_options: Option<&T>,
    provider: &str,
) -> Result<Option<Value>>
where
    T: ProviderOptionsSource + ?Sized,
{
    let Some(provider_options) = provider_options else {
        return Ok(None);
    };
    let provider_options = provider_options.provider_options_value();

    let Some(obj) = provider_options.as_object() else {
        return Ok(Some(provider_options.clone()));
    };

    if !provider_options_object_is_bucketed(obj) {
        return Ok(Some(provider_options.clone()));
    }

    let mut merged = Map::<String, Value>::new();
    let mut has_any = false;

    if let Some(value) = obj.get("*") {
        let Some(bucket) = value.as_object() else {
            return Err(DittoError::InvalidResponse(
                "invalid provider_options: bucket \"*\" must be a JSON object".to_string(),
            ));
        };
        for (key, value) in bucket {
            merged.insert(key.clone(), value.clone());
        }
        has_any = true;
    }

    if let Some(value) = obj.get(provider) {
        let Some(bucket) = value.as_object() else {
            return Err(DittoError::InvalidResponse(format!(
                "invalid provider_options: bucket {provider:?} must be a JSON object"
            )));
        };
        for (key, value) in bucket {
            merged.insert(key.clone(), value.clone());
        }
        has_any = true;
    } else if let Some(alias) = provider_bucket_alias_key(provider)
        && let Some(value) = obj.get(alias)
    {
        let Some(bucket) = value.as_object() else {
            return Err(DittoError::InvalidResponse(format!(
                "invalid provider_options: bucket {alias:?} must be a JSON object"
            )));
        };
        for (key, value) in bucket {
            merged.insert(key.clone(), value.clone());
        }
        has_any = true;
    }

    if !has_any {
        return Ok(None);
    }

    Ok(Some(Value::Object(merged)))
}

#[cfg(any(
    feature = "anthropic",
    feature = "bedrock",
    feature = "cohere",
    feature = "google",
    feature = "openai",
    feature = "openai-compatible",
    feature = "vertex",
))]
pub(crate) fn merge_provider_options_into_body(
    body: &mut Map<String, Value>,
    options: Option<&Value>,
    reserved_keys: &[&str],
    feature: &str,
    warnings: &mut Vec<Warning>,
) {
    let Some(options) = options else {
        return;
    };
    let Some(obj) = options.as_object() else {
        warnings.push(Warning::Unsupported {
            feature: feature.to_string(),
            details: Some("expected provider_options to be a JSON object".to_string()),
        });
        return;
    };

    for (key, value) in obj {
        if reserved_keys.contains(&key.as_str()) {
            continue;
        }

        if let Some(existing) = body.get_mut(key) {
            match (existing.as_object_mut(), value.as_object()) {
                (Some(existing_obj), Some(value_obj)) => {
                    for (nested_key, nested_value) in value_obj {
                        if existing_obj.contains_key(nested_key) {
                            warnings.push(Warning::Compatibility {
                                feature: feature.to_string(),
                                details: format!(
                                    "provider_options overrides {key}.{nested_key}; ignoring override"
                                ),
                            });
                            continue;
                        }
                        existing_obj.insert(nested_key.clone(), nested_value.clone());
                    }
                }
                _ => warnings.push(Warning::Compatibility {
                    feature: feature.to_string(),
                    details: format!("provider_options overrides {key}; ignoring override"),
                }),
            }
            continue;
        }

        body.insert(key.clone(), value.clone());
    }
}
