use std::{net::SocketAddr, path::PathBuf};

use model_rocket::config::Config;

fn config(listen: &str, bearer: &str) -> Result<Config, Box<dyn std::error::Error>> {
    Ok(Config::test_fixture(
        listen.parse::<SocketAddr>()?,
        None,
        &PathBuf::from("/usr/bin/true"),
        std::env::current_dir()?,
        bearer.to_owned(),
    )?)
}

#[test]
fn non_loopback_listener_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
    let error = config("0.0.0.0:8765", "01234567890123456789012345678901")
        .err()
        .ok_or_else(|| std::io::Error::other("non-loopback listener must fail"))?;
    assert!(error.to_string().contains("loopback"));
    Ok(())
}

#[test]
fn short_local_bearer_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
    let error = config("127.0.0.1:8765", "too-short")
        .err()
        .ok_or_else(|| std::io::Error::other("short bearer must fail"))?;
    assert!(error.to_string().contains("at least 32 bytes"));
    Ok(())
}

#[test]
fn alternate_gpt_model_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
    let configured = config("127.0.0.1:8765", "01234567890123456789012345678901")?;
    assert!(configured.catalogue().route_for("gpt-other").is_none());
    Ok(())
}

#[test]
fn relative_codex_binary_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
    let error = Config::test_fixture(
        "127.0.0.1:8765".parse::<SocketAddr>()?,
        None,
        &PathBuf::from("codex"),
        std::env::current_dir()?,
        "01234567890123456789012345678901".to_owned(),
    )
    .err()
    .ok_or_else(|| std::io::Error::other("relative Codex binary must fail"))?;
    assert!(error.to_string().contains("absolute path"));
    Ok(())
}
