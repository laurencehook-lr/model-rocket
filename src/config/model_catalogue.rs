use std::{
    fs::File,
    io::{Read, Take},
    path::Path,
    sync::Arc,
};

use serde::Deserialize;

use crate::domain::{
    BridgeError, ClaudeModelId, ContextTokens, ModelCatalogue, ModelDefinition, ModelRoute,
    ReasoningEffort, ServiceTier,
};

const SCHEMA_VERSION: u32 = 1;
const MAX_CONFIG_BYTES: u64 = 256 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogueDocument {
    schema_version: u32,
    canonical_route: String,
    models: Vec<ModelDocument>,
    routes: Vec<RouteDocument>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelDocument {
    id: String,
    display_name: String,
    description: String,
    context_tokens: u64,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum DeliveryDocument {
    Standard,
    Fast,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ReasoningDocument {
    Low,
    High,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RouteDocument {
    id: String,
    display_name: String,
    description: String,
    model: String,
    delivery: DeliveryDocument,
    reasoning: ReasoningDocument,
}

/// Loads a strict versioned model catalogue from one startup snapshot.
///
/// # Errors
///
/// Returns an error when the file is unavailable, oversized, malformed, or semantically invalid.
pub(super) fn load(path: &Path) -> Result<Arc<ModelCatalogue>, BridgeError> {
    let file = File::open(path).map_err(|error| {
        BridgeError::configuration(format!(
            "cannot open model catalogue {}: {error}",
            path.display()
        ))
    })?;
    let metadata = file.metadata().map_err(|error| {
        BridgeError::configuration(format!(
            "cannot inspect model catalogue {}: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(BridgeError::configuration(format!(
            "model catalogue is not a regular file: {}",
            path.display()
        )));
    }
    if metadata.len() > MAX_CONFIG_BYTES {
        return Err(BridgeError::configuration(format!(
            "model catalogue exceeds {MAX_CONFIG_BYTES} bytes: {}",
            path.display()
        )));
    }
    let mut contents = Vec::new();
    let mut limited: Take<File> = file.take(MAX_CONFIG_BYTES + 1);
    limited.read_to_end(&mut contents).map_err(|error| {
        BridgeError::configuration(format!(
            "cannot read model catalogue {}: {error}",
            path.display()
        ))
    })?;
    if contents.len() as u64 > MAX_CONFIG_BYTES {
        return Err(BridgeError::configuration(format!(
            "model catalogue exceeds {MAX_CONFIG_BYTES} bytes: {}",
            path.display()
        )));
    }
    decode(&contents, path)
}

fn decode(contents: &[u8], path: &Path) -> Result<Arc<ModelCatalogue>, BridgeError> {
    let document: CatalogueDocument = serde_json::from_slice(contents).map_err(|error| {
        BridgeError::configuration(format!(
            "cannot parse model catalogue {}: {error}",
            path.display()
        ))
    })?;
    if document.schema_version != SCHEMA_VERSION {
        return Err(BridgeError::configuration(format!(
            "unsupported model catalogue schema_version {}; expected {SCHEMA_VERSION}",
            document.schema_version
        )));
    }
    let models = document
        .models
        .into_iter()
        .map(|model| {
            ModelDefinition::new(
                model.id,
                model.display_name,
                model.description,
                ContextTokens::new(model.context_tokens)?,
            )
        })
        .collect::<Result<Vec<_>, BridgeError>>()?;
    let routes = document
        .routes
        .into_iter()
        .map(|route| ModelRoute {
            claude_model: ClaudeModelId::new(route.id),
            display_name: Arc::from(route.display_name),
            description: Arc::from(route.description),
            codex_model: crate::domain::CodexModelId::new(route.model),
            service_tier: match route.delivery {
                DeliveryDocument::Standard => ServiceTier::Standard,
                DeliveryDocument::Fast => ServiceTier::Fast,
            },
            reasoning_effort: match route.reasoning {
                ReasoningDocument::Low => ReasoningEffort::Low,
                ReasoningDocument::High => ReasoningEffort::High,
            },
        })
        .collect();
    let canonical_route = ClaudeModelId::new(document.canonical_route);
    ModelCatalogue::new(models, routes, &canonical_route).map(Arc::new)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    use serde_json::{Value, json};

    use super::{decode, load};

    #[test]
    fn loads_a_two_model_catalogue_and_derives_minimum_context()
    -> Result<(), Box<dyn std::error::Error>> {
        let path = fixture_path("two-models");
        fs::write(
            &path,
            r#"{
              "schema_version": 1,
              "canonical_route": "anthropic-model-rocket-gpt-a-high",
              "models": [
                {"id":"gpt-a","display_name":"A","description":"A model","context_tokens":1000000},
                {"id":"gpt-b","display_name":"B","description":"B model","context_tokens":250000}
              ],
              "routes": [
                {"id":"anthropic-model-rocket-gpt-a-high","display_name":"A High","description":"A high","model":"gpt-a","delivery":"standard","reasoning":"high"},
                {"id":"anthropic-model-rocket-gpt-b-low","display_name":"B Low","description":"B low","model":"gpt-b","delivery":"fast","reasoning":"low"}
              ]
            }"#,
        )?;
        let catalogue = load(&path)?;
        fs::remove_file(path)?;
        assert_eq!(catalogue.models().len(), 2);
        assert_eq!(catalogue.routes().len(), 2);
        assert_eq!(catalogue.minimum_context_tokens().get(), 250_000);
        Ok(())
    }

    #[test]
    fn rejects_unknown_fields() -> Result<(), Box<dyn std::error::Error>> {
        let path = fixture_path("unknown-field");
        fs::write(
            &path,
            r#"{"schema_version":1,"canonical_route":"x","models":[],"routes":[],"unexpected":true}"#,
        )?;
        let error = load(&path).err().ok_or("unknown field was accepted")?;
        fs::remove_file(path)?;
        assert!(error.to_string().contains("unknown field"));
        Ok(())
    }

    #[test]
    fn rejects_every_cross_catalogue_invariant() -> Result<(), Box<dyn std::error::Error>> {
        let mut document = valid_document();
        *document
            .pointer_mut("/schema_version")
            .ok_or("schema version missing")? = json!(2);
        assert_invalid(&document, "schema_version")?;

        let mut document = valid_document();
        duplicate_first(&mut document, "models")?;
        assert_invalid(&document, "duplicate model id")?;

        let mut document = valid_document();
        duplicate_first(&mut document, "routes")?;
        assert_invalid(&document, "duplicate route id")?;

        let mut document = valid_document();
        *document
            .pointer_mut("/routes/0/model")
            .ok_or("route model missing")? = json!("gpt-missing");
        assert_invalid(&document, "references unknown model")?;

        let mut document = valid_document();
        document
            .get_mut("models")
            .and_then(Value::as_array_mut)
            .ok_or("models missing")?
            .push(json!({
                "id": "gpt-unused",
                "display_name": "Unused",
                "description": "Unused model",
                "context_tokens": 1000
            }));
        assert_invalid(&document, "not referenced")?;

        let mut document = valid_document();
        *document
            .pointer_mut("/canonical_route")
            .ok_or("canonical route missing")? = json!("anthropic-model-rocket-missing");
        assert_invalid(&document, "is not a configured route")?;

        let mut document = valid_document();
        *document
            .pointer_mut("/routes/0/id")
            .ok_or("route id missing")? = json!("gpt-raw-route");
        assert_invalid(&document, "Claude route id must start")?;

        let mut document = valid_document();
        *document
            .pointer_mut("/models/0/context_tokens")
            .ok_or("model context missing")? = json!(0);
        assert_invalid(&document, "context_tokens")?;
        Ok(())
    }

    #[test]
    fn rejects_unknown_delivery_and_reasoning_values() -> Result<(), Box<dyn std::error::Error>> {
        for (field, value) in [("delivery", "turbo"), ("reasoning", "medium")] {
            let mut document = valid_document();
            document
                .pointer_mut("/routes/0")
                .and_then(Value::as_object_mut)
                .ok_or("route missing")?
                .insert(field.to_owned(), json!(value));
            let encoded = serde_json::to_vec(&document)?;
            assert!(decode(&encoded, Path::new("test.json")).is_err());
        }
        Ok(())
    }

    #[test]
    fn rejects_oversized_catalogue_file() -> Result<(), Box<dyn std::error::Error>> {
        let path = fixture_path("oversized");
        fs::write(&path, vec![b' '; 256 * 1024 + 1])?;
        let error = load(&path).err().ok_or("oversized file was accepted")?;
        fs::remove_file(path)?;
        assert!(error.to_string().contains("exceeds 262144 bytes"));
        Ok(())
    }

    #[test]
    fn enforces_model_and_route_count_limits() -> Result<(), Box<dyn std::error::Error>> {
        let mut too_many_models = valid_document();
        let mut models = Vec::new();
        let mut routes = Vec::new();
        for index in 0..65 {
            models.push(json!({
                "id": format!("gpt-{index}"),
                "display_name": format!("Model {index}"),
                "description": format!("Model {index}"),
                "context_tokens": 1000
            }));
            routes.push(json!({
                "id": format!("anthropic-model-rocket-gpt-{index}"),
                "display_name": format!("Route {index}"),
                "description": format!("Route {index}"),
                "model": format!("gpt-{index}"),
                "delivery": "standard",
                "reasoning": "low"
            }));
        }
        *too_many_models.get_mut("models").ok_or("models missing")? = Value::Array(models);
        *too_many_models.get_mut("routes").ok_or("routes missing")? = Value::Array(routes);
        assert_invalid(&too_many_models, "between 1 and 64")?;

        let mut too_many_routes = valid_document();
        let routes = too_many_routes
            .get_mut("routes")
            .and_then(Value::as_array_mut)
            .ok_or("routes missing")?;
        routes.clear();
        for index in 0..257 {
            routes.push(json!({
                "id": format!("anthropic-model-rocket-gpt-a-{index}"),
                "display_name": format!("Route {index}"),
                "description": format!("Route {index}"),
                "model": "gpt-a",
                "delivery": "standard",
                "reasoning": "low"
            }));
        }
        assert_invalid(&too_many_routes, "between 1 and 256")?;
        Ok(())
    }

    #[test]
    fn enforces_context_identifier_and_label_boundaries() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut maximum_context = valid_document();
        *maximum_context
            .pointer_mut("/models/0/context_tokens")
            .ok_or("model context missing")? = json!(4_000_000);
        decode(
            &serde_json::to_vec(&maximum_context)?,
            Path::new("test.json"),
        )?;

        let mut excessive_context = maximum_context;
        *excessive_context
            .pointer_mut("/models/0/context_tokens")
            .ok_or("model context missing")? = json!(4_000_001);
        assert_invalid(&excessive_context, "between 1 and 4000000")?;

        for (pointer, invalid, expected) in [
            ("/models/0/id", json!("gpt-BAD"), "lowercase ASCII"),
            (
                "/routes/0/id",
                json!("anthropic-model-rocket-gpt a"),
                "lowercase ASCII",
            ),
            ("/models/0/display_name", json!(""), "non-empty"),
            (
                "/models/0/description",
                json!("contains\ncontrol"),
                "control characters",
            ),
            (
                "/routes/0/display_name",
                json!("x".repeat(513)),
                "at most 512 bytes",
            ),
        ] {
            let mut document = valid_document();
            *document.pointer_mut(pointer).ok_or("field missing")? = invalid;
            assert_invalid(&document, expected)?;
        }
        Ok(())
    }

    fn valid_document() -> Value {
        json!({
            "schema_version": 1,
            "canonical_route": "anthropic-model-rocket-gpt-a-high",
            "models": [{
                "id": "gpt-a",
                "display_name": "A",
                "description": "A model",
                "context_tokens": 1_000_000
            }],
            "routes": [{
                "id": "anthropic-model-rocket-gpt-a-high",
                "display_name": "A High",
                "description": "A high route",
                "model": "gpt-a",
                "delivery": "standard",
                "reasoning": "high"
            }]
        })
    }

    fn duplicate_first(
        document: &mut Value,
        collection: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let entries = document
            .get_mut(collection)
            .and_then(Value::as_array_mut)
            .ok_or("collection missing")?;
        let first = entries.first().ok_or("collection empty")?.clone();
        entries.push(first);
        Ok(())
    }

    fn assert_invalid(document: &Value, expected: &str) -> Result<(), Box<dyn std::error::Error>> {
        let encoded = serde_json::to_vec(document)?;
        let error = decode(&encoded, Path::new("test.json"))
            .err()
            .ok_or("invalid catalogue was accepted")?;
        assert!(
            error.to_string().contains(expected),
            "unexpected catalogue error: {error}"
        );
        Ok(())
    }

    fn fixture_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "model-rocket-catalogue-{label}-{}-{}.json",
            std::process::id(),
            uuid::Uuid::now_v7()
        ))
    }
}
