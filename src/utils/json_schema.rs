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

    if let Some(max_length) = input.get("maxLength").and_then(Value::as_u64) {
        out.insert("maxLength".to_string(), Value::Number(max_length.into()));
    }

    if let Some(pattern) = input.get("pattern").and_then(Value::as_str) {
        out.insert("pattern".to_string(), Value::String(pattern.to_string()));
    }

    if let Some(min_items) = input.get("minItems").and_then(Value::as_u64) {
        out.insert("minItems".to_string(), Value::Number(min_items.into()));
    }

    if let Some(max_items) = input.get("maxItems").and_then(Value::as_u64) {
        out.insert("maxItems".to_string(), Value::Number(max_items.into()));
    }

    if let Some(unique_items) = input.get("uniqueItems").and_then(Value::as_bool) {
        out.insert("uniqueItems".to_string(), Value::Bool(unique_items));
    }

    if let Some(min_properties) = input.get("minProperties").and_then(Value::as_u64) {
        out.insert(
            "minProperties".to_string(),
            Value::Number(min_properties.into()),
        );
    }

    if let Some(max_properties) = input.get("maxProperties").and_then(Value::as_u64) {
        out.insert(
            "maxProperties".to_string(),
            Value::Number(max_properties.into()),
        );
    }

    if let Some(additional_properties) = input.get("additionalProperties") {
        let mapped = match additional_properties {
            Value::Bool(value) => Value::Bool(*value),
            Value::Object(_) | Value::Array(_) => {
                convert_json_schema_to_openapi_schema(additional_properties, false)
                    .unwrap_or(Value::Null)
            }
            other => other.clone(),
        };
        out.insert("additionalProperties".to_string(), mapped);
    }

    if let Some(default_value) = input.get("default") {
        out.insert("default".to_string(), default_value.clone());
    }

    if let Some(title) = input.get("title").and_then(Value::as_str) {
        out.insert("title".to_string(), Value::String(title.to_string()));
    }

    if let Some(Value::Number(n)) = input.get("minimum") {
        out.insert("minimum".to_string(), Value::Number(n.clone()));
    }

    if let Some(Value::Number(n)) = input.get("maximum") {
        out.insert("maximum".to_string(), Value::Number(n.clone()));
    }

    if let Some(Value::Number(n)) = input.get("multipleOf") {
        out.insert("multipleOf".to_string(), Value::Number(n.clone()));
    }

    match input.get("exclusiveMinimum") {
        Some(Value::Number(n)) => {
            out.insert("exclusiveMinimum".to_string(), Value::Bool(true));
            out.insert("minimum".to_string(), Value::Number(n.clone()));
        }
        Some(Value::Bool(v)) => {
            out.insert("exclusiveMinimum".to_string(), Value::Bool(*v));
        }
        _ => {}
    }

    match input.get("exclusiveMaximum") {
        Some(Value::Number(n)) => {
            out.insert("exclusiveMaximum".to_string(), Value::Bool(true));
            out.insert("maximum".to_string(), Value::Number(n.clone()));
        }
        Some(Value::Bool(v)) => {
            out.insert("exclusiveMaximum".to_string(), Value::Bool(*v));
        }
        _ => {}
    }

    Some(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

    #[test]
    fn converts_string_constraints() {
        let schema = json!({
            "type": "string",
            "minLength": 1,
            "maxLength": 10,
            "pattern": "^[a-z]+$"
        });
        assert_eq!(
            convert_json_schema_to_openapi_schema(&schema, true),
            Some(json!({
                "type": "string",
                "minLength": 1,
                "maxLength": 10,
                "pattern": "^[a-z]+$"
            }))
        );
    }

    #[test]
    fn converts_number_constraints_and_exclusive_bounds() {
        let schema = json!({
            "type": "number",
            "minimum": 0,
            "exclusiveMaximum": 5,
            "multipleOf": 0.5
        });
        assert_eq!(
            convert_json_schema_to_openapi_schema(&schema, true),
            Some(json!({
                "type": "number",
                "minimum": 0,
                "maximum": 5,
                "exclusiveMaximum": true,
                "multipleOf": 0.5
            }))
        );
    }

    #[test]
    fn converts_object_constraints_and_additional_properties() {
        let schema = json!({
            "type": "object",
            "title": "args",
            "minProperties": 1,
            "maxProperties": 2,
            "properties": {
                "a": { "type": "string", "default": "x" }
            },
            "additionalProperties": false
        });
        assert_eq!(
            convert_json_schema_to_openapi_schema(&schema, true),
            Some(json!({
                "title": "args",
                "type": "object",
                "minProperties": 1,
                "maxProperties": 2,
                "properties": {
                    "a": { "type": "string", "default": "x" }
                },
                "additionalProperties": false
            }))
        );
    }

    #[test]
    fn converts_array_constraints() {
        let schema = json!({
            "type": "array",
            "items": { "type": "string" },
            "minItems": 1,
            "maxItems": 3,
            "uniqueItems": true
        });
        assert_eq!(
            convert_json_schema_to_openapi_schema(&schema, true),
            Some(json!({
                "type": "array",
                "items": { "type": "string" },
                "minItems": 1,
                "maxItems": 3,
                "uniqueItems": true
            }))
        );
    }
}
