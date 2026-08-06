use std::{
    fs,
    net::SocketAddr,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
};

use model_rocket::domain::AccountMode;
use model_rocket::{
    adapters::outbound::codex::AppServer,
    bootstrap,
    config::{Config, PreflightConfig},
    contracts::codex::restricted_model_catalog,
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn fixture_config(name: &str) -> Result<Config, Box<dyn std::error::Error>> {
    Ok(Config::test_fixture(
        "127.0.0.1:0".parse::<SocketAddr>()?,
        None,
        &fixture(name),
        std::env::current_dir()?,
        "01234567890123456789012345678901".to_owned(),
    )?)
}

struct FixtureCopy(PathBuf);

impl FixtureCopy {
    fn create(source: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let path = std::env::temp_dir().join(format!(
            "model-rocket-executable-replacement-{}",
            uuid::Uuid::now_v7()
        ));
        fs::copy(source, &path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for FixtureCopy {
    fn drop(&mut self) {
        let _permissions = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
        let _removed = fs::remove_file(&self.0);
    }
}

#[tokio::test]
async fn preflight_accepts_managed_chatgpt_and_exact_model()
-> Result<(), Box<dyn std::error::Error>> {
    let config = fixture_config("fake_codex_discovery.py")?;
    let mut server = AppServer::launch_discovery(config.codex_executable()).await?;
    let reports = server.preflight(config.catalogue()).await?;
    let report = reports.first().ok_or("preflight report missing")?;
    assert_eq!(report.account(), AccountMode::ManagedChatGpt);
    assert_eq!(report.model().as_str(), "gpt-5.6-sol");
    Ok(())
}

#[tokio::test]
async fn preflight_rejects_api_key_mode() -> Result<(), Box<dyn std::error::Error>> {
    let config = fixture_config("fake_codex_api_key.sh")?;
    let mut server = AppServer::launch_discovery(config.codex_executable()).await?;
    let error = server
        .preflight(config.catalogue())
        .await
        .err()
        .ok_or_else(|| std::io::Error::other("API-key mode must fail"))?;
    assert!(error.to_string().contains("managed ChatGPT"));
    Ok(())
}

#[tokio::test]
async fn launch_rejects_unpinned_codex_version() -> Result<(), Box<dyn std::error::Error>> {
    let config = fixture_config("fake_codex_wrong_version.sh")?;
    let error = AppServer::launch(config.codex_executable(), Arc::clone(config.catalogue()))
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
async fn launch_revalidates_executable_after_configuration()
-> Result<(), Box<dyn std::error::Error>> {
    let copied = FixtureCopy::create(&fixture("fake_codex.py"))?;
    let config = Config::test_fixture(
        "127.0.0.1:0".parse::<SocketAddr>()?,
        None,
        copied.path(),
        std::env::current_dir()?,
        "01234567890123456789012345678901".to_owned(),
    )?;
    fs::set_permissions(copied.path(), fs::Permissions::from_mode(0o600))?;

    let error = AppServer::launch(config.codex_executable(), Arc::clone(config.catalogue()))
        .await
        .err()
        .ok_or_else(|| std::io::Error::other("replaced executable must fail before spawn"))?;
    assert!(error.to_string().contains("must be executable"));
    Ok(())
}

#[tokio::test]
async fn launch_revalidates_executable_immediately_after_version_probe()
-> Result<(), Box<dyn std::error::Error>> {
    let copied = FixtureCopy::create(&fixture("fake_codex_replace_on_version.sh"))?;
    let config = Config::test_fixture(
        "127.0.0.1:0".parse::<SocketAddr>()?,
        None,
        copied.path(),
        std::env::current_dir()?,
        "01234567890123456789012345678901".to_owned(),
    )?;

    let error = AppServer::launch(config.codex_executable(), Arc::clone(config.catalogue()))
        .await
        .err()
        .ok_or_else(|| std::io::Error::other("version-time replacement must fail before spawn"))?;
    assert!(error.to_string().contains("must be executable"));
    Ok(())
}

#[tokio::test]
async fn fixture_configuration_rejects_relative_codex_binary_path()
-> Result<(), Box<dyn std::error::Error>> {
    let error = Config::test_fixture(
        "127.0.0.1:0".parse::<SocketAddr>()?,
        None,
        &PathBuf::from("codex"),
        std::env::current_dir()?,
        "01234567890123456789012345678901".to_owned(),
    )
    .err()
    .ok_or_else(|| std::io::Error::other("relative Codex path must fail"))?;
    assert!(error.to_string().contains("absolute path"));
    Ok(())
}

#[tokio::test]
async fn app_server_child_has_no_api_key_environment() -> Result<(), Box<dyn std::error::Error>> {
    let config = fixture_config("fake_codex_assert_clean.sh")?;
    let mut server =
        AppServer::launch(config.codex_executable(), Arc::clone(config.catalogue())).await?;
    let reports = server.preflight(config.catalogue()).await?;
    assert_eq!(
        reports.first().ok_or("preflight report missing")?.account(),
        AccountMode::ManagedChatGpt
    );
    Ok(())
}

#[test]
fn restricted_model_catalog_exposes_no_codex_tools() -> Result<(), Box<dyn std::error::Error>> {
    let config = fixture_config("fake_codex.py")?;
    let catalog = restricted_model_catalog(config.catalogue())?;
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
    assert_eq!(
        model
            .pointer("/service_tiers/0/id")
            .and_then(serde_json::Value::as_str),
        Some("priority")
    );
    assert!(
        model
            .get("default_service_tier")
            .is_some_and(serde_json::Value::is_null)
    );
    Ok(())
}

#[tokio::test]
async fn explicit_preflight_checks_every_configured_model_and_fails_as_one_unit()
-> Result<(), Box<dyn std::error::Error>> {
    let catalogue_path = std::env::temp_dir().join(format!(
        "model-rocket-two-model-preflight-{}.json",
        uuid::Uuid::now_v7()
    ));
    fs::write(
        &catalogue_path,
        r#"{
          "schema_version": 1,
          "canonical_route": "anthropic-model-rocket-gpt-5.6-sol-high",
          "models": [
            {"id":"gpt-5.6-sol","display_name":"Sol","description":"Sol model","context_tokens":272000},
            {"id":"gpt-unavailable","display_name":"Unavailable","description":"Unavailable model","context_tokens":128000}
          ],
          "routes": [
            {"id":"anthropic-model-rocket-gpt-5.6-sol-high","display_name":"Sol High","description":"Sol route","model":"gpt-5.6-sol","delivery":"standard","reasoning":"high"},
            {"id":"anthropic-model-rocket-gpt-unavailable-low","display_name":"Unavailable Low","description":"Unavailable route","model":"gpt-unavailable","delivery":"standard","reasoning":"low"}
          ]
        }"#,
    )?;
    let config =
        PreflightConfig::test_fixture(&fixture("fake_codex_discovery.py"), &catalogue_path)?;
    let result = bootstrap::preflight(&config).await;
    fs::remove_file(catalogue_path)?;
    let error = result
        .err()
        .ok_or("unavailable configured model was accepted")?;
    assert!(error.to_string().contains("gpt-unavailable"));
    Ok(())
}

#[tokio::test]
async fn explicit_preflight_rejects_repeated_model_list_cursor()
-> Result<(), Box<dyn std::error::Error>> {
    let catalogue_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("config/model-routes.json");
    let config =
        PreflightConfig::test_fixture(&fixture("fake_codex_repeated_cursor.py"), &catalogue_path)?;
    let error = bootstrap::preflight(&config)
        .await
        .err()
        .ok_or("repeated model/list cursor was accepted")?;
    assert!(error.to_string().contains("repeated cursor loop"));
    Ok(())
}

#[tokio::test]
async fn explicit_preflight_enforces_model_list_page_bound()
-> Result<(), Box<dyn std::error::Error>> {
    let catalogue_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("config/model-routes.json");
    let config =
        PreflightConfig::test_fixture(&fixture("fake_codex_unbounded_pages.py"), &catalogue_path)?;
    let error = bootstrap::preflight(&config)
        .await
        .err()
        .ok_or("unbounded model/list pagination was accepted")?;
    assert!(error.to_string().contains("exceeded the page limit"));
    Ok(())
}
