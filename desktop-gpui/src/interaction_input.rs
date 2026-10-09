//! Pure native elicitation projection; the backend remains the response authority.
use serde_json::{Map, Value};
use std::collections::HashMap;

pub fn fields(schema: &Value) -> Result<&Map<String, Value>, String> {
    let fields = schema["properties"]
        .as_object()
        .ok_or("Unsupported form schema")?;
    for (name, field) in fields {
        if !matches!(
            field["type"].as_str(),
            Some("string" | "number" | "integer" | "boolean")
        ) {
            return Err(format!("Unsupported field type: {name}"));
        }
        if field.get("enum").is_some_and(|v| !v.is_array()) {
            return Err(format!("Unsupported choices: {name}"));
        }
    }
    Ok(fields)
}

pub fn response(
    schema: &Value,
    text: &HashMap<String, String>,
    choices: &HashMap<String, Value>,
) -> Result<Value, String> {
    let mut result = Map::new();
    for (name, field) in fields(schema)? {
        let value = if let Some(choice) = choices.get(name) {
            Some(choice.clone())
        } else if let Some(raw) = text.get(name).filter(|v| !v.is_empty()) {
            Some(match field["type"].as_str() {
                Some("string") => Value::String(raw.clone()),
                Some("integer" | "number") => {
                    let parsed: Value = serde_json::from_str(raw.trim())
                        .map_err(|_| format!("Invalid number: {name}"))?;
                    if !parsed.is_number() {
                        return Err(format!("Invalid number: {name}"));
                    }
                    parsed
                }
                _ => return Err(format!("Choose a value: {name}")),
            })
        } else {
            None
        };
        let Some(value) = value else {
            if schema["required"]
                .as_array()
                .is_some_and(|names| names.iter().any(|v| v.as_str() == Some(name)))
            {
                return Err(format!("Missing required field: {name}"));
            }
            continue;
        };
        let valid = match field["type"].as_str() {
            Some("string") => value.is_string(),
            Some("boolean") => value.is_boolean(),
            Some("integer") => value.is_i64() || value.is_u64(),
            Some("number") => value.is_number(),
            _ => false,
        };
        if !valid {
            return Err(format!("Invalid field: {name}"));
        }
        if field["enum"]
            .as_array()
            .is_some_and(|options| !options.contains(&value))
        {
            return Err(format!("Invalid choice: {name}"));
        }
        result.insert(name.clone(), value);
    }
    let response = Value::Object(result);
    if response.to_string().len() > 64 * 1024 {
        return Err("Input response is too large.".into());
    }
    Ok(response)
}

pub fn safe_url(raw: &str) -> Result<String, String> {
    let url = url::Url::parse(raw).map_err(|_| "Invalid browser-step URL")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Only HTTP(S) addresses without embedded credentials can be opened.".into());
    }
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[::core::prelude::v1::test]
    fn primitive_values_preserve_types_and_omit_optional_fields() {
        let schema = json!({"properties":{"name":{"type":"string"},"count":{"type":"integer"},"enabled":{"type":"boolean"},"optional":{"type":"number"}},"required":["count"]});
        let text = HashMap::from([
            ("name".into(), "  literal  ".into()),
            ("count".into(), "42".into()),
        ]);
        assert_eq!(
            response(
                &schema,
                &text,
                &HashMap::from([("enabled".into(), json!(false))])
            )
            .unwrap(),
            json!({"name":"  literal  ","count":42,"enabled":false})
        );
    }
    #[::core::prelude::v1::test]
    fn rejects_fractional_integer_nonfinite_and_missing_required() {
        let schema = json!({"properties":{"n":{"type":"integer"}},"required":["n"]});
        for raw in ["", "1.5", "NaN", "true"] {
            assert!(
                response(
                    &schema,
                    &HashMap::from([("n".into(), raw.into())]),
                    &HashMap::new()
                )
                .is_err()
            );
        }
    }
    #[::core::prelude::v1::test]
    fn typed_enum_does_not_coerce_and_supports_empty_string_choice() {
        let schema = json!({"properties":{"n":{"type":"string","enum":[""]}},"required":["n"]});
        assert_eq!(
            response(
                &schema,
                &HashMap::new(),
                &HashMap::from([("n".into(), json!(""))])
            )
            .unwrap(),
            json!({"n":""})
        );
        assert!(
            response(
                &schema,
                &HashMap::new(),
                &HashMap::from([("n".into(), json!(0))])
            )
            .is_err()
        );
    }
    #[::core::prelude::v1::test]
    fn rejects_nested_schema_and_oversized_response() {
        assert!(fields(&json!({"properties":{"nested":{"type":"object"}}})).is_err());
        assert!(
            response(
                &json!({"properties":{"s":{"type":"string"}}}),
                &HashMap::from([("s".into(), "x".repeat(65536))]),
                &HashMap::new()
            )
            .is_err()
        );
    }
    #[::core::prelude::v1::test]
    fn browser_step_rejects_active_schemes_and_credentials() {
        for raw in [
            "javascript:alert(1)",
            "file:///tmp/file",
            "https://user:password@example.com",
            "relative",
        ] {
            assert!(safe_url(raw).is_err());
        }
        assert_eq!(
            safe_url("https://example.com/step").unwrap(),
            "https://example.com/step"
        );
    }
}
