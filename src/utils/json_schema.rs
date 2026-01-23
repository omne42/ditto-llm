use serde_json::{Map, Value};

fn is_empty_object_schema(schema: &Value) -> bool {
    let Value::Object(obj) = schema else {
        return false;
    };
    if obj.get("type").and_then(Value::as_str) != Some("object") {
        return false;
    }
    let properties_len = obj
        .get("properties")
        .and_then(Value::as_object)
        .map(|p| p.len())
        .unwrap_or(0);
    if properties_len != 0 {
        return false;
    }
    matches!(
        obj.get("additionalProperties"),
        None | Some(Value::Bool(false))
    )
}

pub fn convert_json_schema_to_openapi_schema(schema: &Value, is_root: bool) -> Option<Value> {
    if schema.is_null() {
        return None;
    }

    if is_empty_object_schema(schema) {
        if is_root {
            return None;
        }
        let mut out = Map::new();
        out.insert("type".to_string(), Value::String("object".to_string()));
        if let Some(description) = schema.get("description").and_then(Value::as_str) {
            out.insert(
                "description".to_string(),
                Value::String(description.to_string()),
            );
        }
        return Some(Value::Object(out));
    }

    if let Value::Bool(_) = schema {
        return Some(serde_json::json!({ "type": "boolean", "properties": {} }));
    }

    let Value::Object(input) = schema else {
        return Some(schema.clone());
    };

    let mut out = Map::<String, Value>::new();

    if let Some(description) = input.get("description").and_then(Value::as_str) {
        out.insert(
            "description".to_string(),
            Value::String(description.to_string()),
        );
    }

    if let Some(required) = input.get("required").and_then(Value::as_array) {
        out.insert("required".to_string(), Value::Array(required.clone()));
    }

    if let Some(format) = input.get("format").and_then(Value::as_str) {
        out.insert("format".to_string(), Value::String(format.to_string()));
    }

    if let Some(const_value) = input.get("const") {
        out.insert("enum".to_string(), Value::Array(vec![const_value.clone()]));
    }

    if let Some(schema_type) = input.get("type") {
        match schema_type {
            Value::String(type_name) => {
                out.insert("type".to_string(), Value::String(type_name.clone()));
            }
            Value::Array(types) => {
                let mut has_null = false;
                let mut non_null = Vec::<Value>::new();
                for entry in types {
                    if entry.as_str() == Some("null") {
                        has_null = true;
                    } else if let Some(t) = entry.as_str() {
                        non_null.push(serde_json::json!({ "type": t }));
                    }
                }

                if non_null.is_empty() {
                    out.insert("type".to_string(), Value::String("null".to_string()));
                } else {
                    out.insert("anyOf".to_string(), Value::Array(non_null));
                    if has_null {
                        out.insert("nullable".to_string(), Value::Bool(true));
                    }
                }
            }
            _ => {}
        }
    }

    if let Some(enum_values) = input.get("enum").and_then(Value::as_array) {
        out.insert("enum".to_string(), Value::Array(enum_values.clone()));
    }

    if let Some(properties) = input.get("properties").and_then(Value::as_object) {
        let mut mapped = Map::<String, Value>::new();
        for (key, value) in properties {
            let converted =
                convert_json_schema_to_openapi_schema(value, false).unwrap_or(Value::Null);
            mapped.insert(key.clone(), converted);
        }
        out.insert("properties".to_string(), Value::Object(mapped));
    }

    if let Some(items) = input.get("items") {
        let mapped = match items {
            Value::Array(values) => Value::Array(
                values
                    .iter()
                    .map(|item| {
                        convert_json_schema_to_openapi_schema(item, false).unwrap_or(Value::Null)
                    })
                    .collect(),
            ),
            _ => convert_json_schema_to_openapi_schema(items, false).unwrap_or(Value::Null),
        };
        out.insert("items".to_string(), mapped);
    }

    if let Some(all_of) = input.get("allOf").and_then(Value::as_array) {
        out.insert(
            "allOf".to_string(),
            Value::Array(
                all_of
                    .iter()
                    .map(|item| {
                        convert_json_schema_to_openapi_schema(item, false).unwrap_or(Value::Null)
                    })
                    .collect(),
            ),
        );
    }

    if let Some(any_of) = input.get("anyOf").and_then(Value::as_array) {
        let has_null = any_of.iter().any(|schema| {
            schema
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|t| t == "null")
        });

        if has_null {
            let non_null_schemas = any_of
                .iter()
                .filter(|schema| schema.get("type").and_then(Value::as_str) != Some("null"))
                .collect::<Vec<_>>();

            if non_null_schemas.len() == 1 {
                if let Some(Value::Object(obj)) =
                    convert_json_schema_to_openapi_schema(non_null_schemas[0], false)
                {
                    out.insert("nullable".to_string(), Value::Bool(true));
                    for (k, v) in obj {
                        out.insert(k, v);
                    }
                }
            } else {
                out.insert(
                    "anyOf".to_string(),
                    Value::Array(
                        non_null_schemas
                            .into_iter()
                            .map(|item| {
                                convert_json_schema_to_openapi_schema(item, false)
                                    .unwrap_or(Value::Null)
                            })
                            .collect(),
                    ),
                );
                out.insert("nullable".to_string(), Value::Bool(true));
            }
        } else {
            out.insert(
                "anyOf".to_string(),
                Value::Array(
                    any_of
                        .iter()
                        .map(|item| {
                            convert_json_schema_to_openapi_schema(item, false)
                                .unwrap_or(Value::Null)
                        })
                        .collect(),
                ),
            );
        }
    }

    if let Some(one_of) = input.get("oneOf").and_then(Value::as_array) {
        out.insert(
            "oneOf".to_string(),
            Value::Array(
                one_of
                    .iter()
                    .map(|item| {
                        convert_json_schema_to_openapi_schema(item, false).unwrap_or(Value::Null)
                    })
                    .collect(),
            ),
        );
    }

    if let Some(min_length) = input.get("minLength").and_then(Value::as_u64) {
        out.insert("minLength".to_string(), Value::Number(min_length.into()));
    }

    Some(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_object_schema_is_removed_at_root() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {}
        });
        assert_eq!(convert_json_schema_to_openapi_schema(&schema, true), None);
    }

    #[test]
    fn empty_object_schema_is_preserved_when_nested() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {}
        });
        assert_eq!(
            convert_json_schema_to_openapi_schema(&schema, false),
            Some(serde_json::json!({ "type": "object" }))
        );
    }
}
