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

    assert!(worker.contains("name: gpt-worktree-worker\n"));
    assert!(worker.contains(&format!(
        "model: {}\n",
        config.catalogue().canonical_route().claude_model.as_str()
    )));
    assert!(worker.contains("isolation: worktree\n"));
    Ok(())
}
