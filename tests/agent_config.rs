#[test]
fn supplied_worker_uses_gpt_in_a_worktree() -> Result<(), Box<dyn std::error::Error>> {
    let config = model_rocket::config::Config::test_fixture(
        "127.0.0.1:0".parse()?,
        None,
        std::path::Path::new("/usr/bin/true"),
        std::env::current_dir()?,
        "01234567890123456789012345678901".to_owned(),
    )?;
    let mut encoded = Vec::new();
    model_rocket::product::write_worker_config(config.catalogue(), &mut encoded)?;
    let worker = String::from_utf8(encoded)?;
    let worker_model = worker
        .lines()
        .find_map(|line| line.strip_prefix("model: "))
        .ok_or("worker model missing")?;
    let route = config
        .catalogue()
        .route_for(worker_model)
        .ok_or("worker model is not a configured route")?;

    assert!(worker.contains("name: gpt-worktree-worker\n"));
    assert_eq!(
        worker_model,
        config.catalogue().canonical_route().claude_model.as_str()
    );
    assert!(route.codex_model.as_str().starts_with("gpt-"));
    assert!(worker.contains("isolation: worktree\n"));
    Ok(())
}
