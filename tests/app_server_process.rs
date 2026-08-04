use std::path::{Path, PathBuf};

use model_rocket::app_server::AppServer;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[tokio::test]
async fn preflight_accepts_managed_chatgpt_and_exact_model()
-> Result<(), Box<dyn std::error::Error>> {
    let mut server = AppServer::launch(&fixture("fake_codex.py")).await?;
    let report = server.preflight("gpt-5.6-sol").await?;
    assert_eq!(report.account_type, "chatgpt");
    assert_eq!(report.model, "gpt-5.6-sol");
    Ok(())
}

#[tokio::test]
async fn preflight_rejects_api_key_mode() -> Result<(), Box<dyn std::error::Error>> {
    let mut server = AppServer::launch(&fixture("fake_codex_api_key.sh")).await?;
    let error = server
        .preflight("gpt-5.6-sol")
        .await
        .err()
        .ok_or_else(|| std::io::Error::other("API-key mode must fail"))?;
    assert!(error.to_string().contains("managed ChatGPT"));
    Ok(())
}

#[tokio::test]
async fn launch_rejects_unpinned_codex_version() -> Result<(), Box<dyn std::error::Error>> {
    let error = AppServer::launch(&fixture("fake_codex_wrong_version.sh"))
        .await
        .err()
        .ok_or_else(|| std::io::Error::other("wrong Codex version must fail"))?;
    assert!(
        error
            .to_string()
            .contains("must be exactly codex-cli 0.146.0")
    );
    Ok(())
}

#[tokio::test]
async fn app_server_child_has_no_api_key_environment() -> Result<(), Box<dyn std::error::Error>> {
    let mut server = AppServer::launch(&fixture("fake_codex_assert_clean.sh")).await?;
    let report = server.preflight("gpt-5.6-sol").await?;
    assert_eq!(report.account_type, "chatgpt");
    Ok(())
}

#[test]
fn restricted_model_catalog_exposes_no_codex_tools() -> Result<(), Box<dyn std::error::Error>> {
    let catalog: serde_json::Value =
        serde_json::from_slice(include_bytes!("../config/codex-models.json"))?;
    let model = catalog
        .get("models")
        .and_then(serde_json::Value::as_array)
        .and_then(|models| models.first())
        .ok_or_else(|| std::io::Error::other("restricted model missing"))?;
    assert_eq!(
        model.get("shell_type").and_then(serde_json::Value::as_str),
        Some("disabled")
    );
    assert!(
        model
            .get("apply_patch_tool_type")
            .is_some_and(serde_json::Value::is_null)
    );
    assert_eq!(
        model
            .get("supports_search_tool")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert!(
        model
            .get("multi_agent_version")
            .is_some_and(serde_json::Value::is_null)
    );
    assert_eq!(
        model.get("tool_mode").and_then(serde_json::Value::as_str),
        Some("direct")
    );
    Ok(())
}
