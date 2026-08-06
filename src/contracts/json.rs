use std::fmt;

use serde_json::{Value, value::RawValue};

use crate::domain::{JsonDocument, JsonObject};

#[derive(Debug)]
pub enum JsonContractError {
    Invalid(serde_json::Error),
    ObjectRequired,
}

impl fmt::Display for JsonContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(error) => write!(formatter, "invalid JSON: {error}"),
            Self::ObjectRequired => formatter.write_str("JSON object required"),
        }
    }
}

impl std::error::Error for JsonContractError {}

impl From<serde_json::Error> for JsonContractError {
    fn from(error: serde_json::Error) -> Self {
        Self::Invalid(error)
    }
}

/// Validates and preserves an arbitrary JSON document.
///
/// # Errors
///
/// Returns an error when the source is not valid JSON.
pub fn document(source: impl Into<String>) -> Result<JsonDocument, JsonContractError> {
    let source = source.into();
    let _validated = serde_json::from_str::<Box<RawValue>>(&source)?;
    Ok(JsonDocument::from_validated(source))
}

/// Validates and serializes a JSON object for the domain boundary.
///
/// # Errors
///
/// Returns an error when `value` is not an object or cannot be encoded.
pub fn object(value: &Value) -> Result<JsonObject, JsonContractError> {
    if !value.is_object() {
        return Err(JsonContractError::ObjectRequired);
    }
    Ok(JsonObject::from_validated(serde_json::to_string(value)?))
}

/// Restores a validated domain JSON object to the serde representation.
///
/// # Errors
///
/// Returns an error when the stored document is invalid or no longer an object.
pub fn object_value(object: &JsonObject) -> Result<Value, JsonContractError> {
    let value: Value = serde_json::from_str(object.as_str())?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(JsonContractError::ObjectRequired)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{document, object, object_value};

    #[test]
    fn document_preserves_exact_valid_json() -> Result<(), Box<dyn std::error::Error>> {
        let source = "{ \"answer\" : 42 }";
        let value = document(source)?;
        assert_eq!(value.as_str(), source);
        Ok(())
    }

    #[test]
    fn document_rejects_invalid_json() {
        assert!(document("{not-json}").is_err());
    }

    #[test]
    fn document_preserves_numbers_outside_serde_value_range()
    -> Result<(), Box<dyn std::error::Error>> {
        let source = r#"{"finite_for_provider":1e400,"precise":123456789012345678901234567890}"#;
        let value = document(source)?;
        assert_eq!(value.as_str(), source);
        Ok(())
    }

    #[test]
    fn object_round_trips_without_framework_storage() -> Result<(), Box<dyn std::error::Error>> {
        let value = json!({"answer": 42});
        let document = object(&value)?;
        assert_eq!(object_value(&document)?, value);
        assert!(object(&json!(["not", "an", "object"])).is_err());
        Ok(())
    }
}
