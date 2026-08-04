#[test]
fn supplied_worker_uses_gpt_in_a_worktree() {
    let worker = include_str!("../config/gpt-worktree-worker.md");

    assert!(worker.contains("name: gpt-worktree-worker\n"));
    assert!(worker.contains("model: gpt-5.6-sol\n"));
    assert!(worker.contains("isolation: worktree\n"));
}
