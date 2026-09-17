//! Minimal JSON-Schema subset validator used for command/capability inputs.
//!
//! Supports: `type`, `required`, `properties`, `additionalProperties`,
//! `enum`, `items`, `minimum`, `maximum`, `minLength`, `pattern` is NOT
//! supported (avoids regex dep). Unknown keywords are ignored.

use serde_json::Value;

pub fn validate(schema: &Value, value: &Value, path: &str) -> Vec<String> {
    let mut errors = Vec::new();
    validate_into(schema, value, path, &mut errors);
    errors
}

fn type_ok(expected: &str, v: &Value) -> bool {
    match expected {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "number" => v.is_number(),
        "integer" => v.as_i64().is_some() || v.as_u64().is_some(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        _ => true,
    }
}

fn validate_into(schema: &Value, value: &Value, path: &str, errors: &mut Vec<String>) {
    if let Some(types) = schema.get("type") {
        let ok = match types {
            Value::String(t) => type_ok(t, value),
            Value::Array(ts) => ts.iter().any(|t| {
                t.as_str().map(|t| type_ok(t, value)).unwrap_or(false)
            }),
            _ => true,
        };
        if !ok {
            errors.push(format!("{path}: expected type {types}, got {}", kind_of(value)));
            return;
        }
    }
    if let Some(e) = schema.get("enum").and_then(|e| e.as_array()) {
        if !e.contains(value) {
            errors.push(format!("{path}: value not in enum"));
        }
    }
    if let Some(min) = schema.get("minimum").and_then(|m| m.as_f64()) {
        if value.as_f64().map(|v| v < min).unwrap_or(false) {
            errors.push(format!("{path}: below minimum {min}"));
        }
    }
    if let Some(max) = schema.get("maximum").and_then(|m| m.as_f64()) {
        if value.as_f64().map(|v| v > max).unwrap_or(false) {
            errors.push(format!("{path}: above maximum {max}"));
        }
    }
    if let Some(min_len) = schema.get("minLength").and_then(|m| m.as_u64()) {
        if value.as_str().map(|s| (s.len() as u64) < min_len).unwrap_or(false) {
            errors.push(format!("{path}: shorter than minLength {min_len}"));
        }
    }
    if let Value::Object(obj) = value {
        if let Some(req) = schema.get("required").and_then(|r| r.as_array()) {
            for key in req.iter().filter_map(|k| k.as_str()) {
                if !obj.contains_key(key) {
                    errors.push(format!("{path}: missing required property `{key}`"));
                }
            }
        }
        if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
            for (k, sub) in props {
                if let Some(v) = obj.get(k) {
                    validate_into(sub, v, &format!("{path}.{k}"), errors);
                }
            }
        }
        if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
            if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
                for k in obj.keys() {
                    if !props.contains_key(k) {
                        errors.push(format!("{path}: unexpected property `{k}`"));
                    }
                }
            }
        }
    }
    if let (Value::Array(items), Some(item_schema)) =
        (value, schema.get("items"))
    {
        for (i, item) in items.iter().enumerate() {
            validate_into(item_schema, item, &format!("{path}[{i}]"), errors);
        }
    }
}

fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates_required_and_type() {
        let schema = json!({
            "type": "object",
            "required": ["name"],
            "properties": { "name": {"type": "string"}, "n": {"type": "integer"} }
        });
        assert!(validate(&schema, &json!({"name": "x", "n": 3}), "$").is_empty());
        let errs = validate(&schema, &json!({"n": "x"}), "$");
        assert!(errs.iter().any(|e| e.contains("missing required")));
        assert!(errs.iter().any(|e| e.contains("expected type")));
    }
}
