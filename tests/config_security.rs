use std::{net::SocketAddr, path::PathBuf};

use model_rocket::config::{Config, DEFAULT_MODEL};

fn config(listen: &str, bearer: &str) -> Result<Config, Box<dyn std::error::Error>> {
    Ok(Config {
        listen: listen.parse::<SocketAddr>()?,
        ready_file: None,
        model: "gpt-5.6-sol".to_owned(),
        codex_bin: PathBuf::from("codex"),
        cwd: PathBuf::from("."),
        bearer: bearer.to_owned(),
    })
}

#[test]
fn non_loopback_listener_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
    let error = config("0.0.0.0:8765", "01234567890123456789012345678901")?
        .validated()
        .err()
        .ok_or_else(|| std::io::Error::other("non-loopback listener must fail"))?;
    assert!(error.to_string().contains("loopback"));
    Ok(())
}

#[test]
fn short_local_bearer_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
    let error = config("127.0.0.1:8765", "too-short")?
        .validated()
        .err()
        .ok_or_else(|| std::io::Error::other("short bearer must fail"))?;
    assert!(error.to_string().contains("at least 32 bytes"));
    Ok(())
}

#[test]
fn alternate_gpt_model_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
    let mut candidate = config("127.0.0.1:0", "01234567890123456789012345678901")?;
    candidate.model = "gpt-other".to_owned();
    let error = candidate
        .validated()
        .err()
        .ok_or_else(|| std::io::Error::other("alternate GPT model must fail"))?;
    assert!(error.to_string().contains(DEFAULT_MODEL));
    Ok(())
}
