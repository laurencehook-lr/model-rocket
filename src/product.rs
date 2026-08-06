//! Single source of truth for Model Rocket's externally visible product contract.

use std::io::{self, Write};

use crate::domain::ModelCatalogue;

pub const MANAGED_ACCOUNT_TYPE: &str = "chatgpt";
pub const CODEX_CLI_VERSION_OUTPUT: &str = "codex-cli 0.146.0";
pub const CLAUDE_CODE_VERSION_OUTPUT: &str = "2.1.223 (Claude Code)";
pub const DEFAULT_CLAUDE_MODEL: &str = "claude-fable-5";
pub const DEFAULT_AVAILABLE_CLAUDE_MODELS: [&str; 4] = ["fable", "opus", "sonnet", "haiku"];
pub const CODEX_BASE_INSTRUCTIONS: &str =
    "Act only as the model inside Claude Code and use only host-supplied dynamic tools.";
pub const CODEX_DEVELOPER_GUARD: &str = "Act as the model inside Claude Code. Respond with assistant text or the supplied dynamic tools only. Never invoke Codex built-in shell, file, web, MCP, collaboration, or user-input tools.";
pub const TRUSTED_CHILD_PATH: &str = "/usr/bin:/bin";
pub const CLAUDE_DISCONNECTED_ERROR: &str = "Claude Code disconnected during generation";
pub const BRIDGE_STARTUP_ATTEMPTS: u16 = 300;

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub const CODEX_NATIVE_SHA256: &str =
    "ae1d3ffe6d48aec6a4dc3f50e7eb8e0d11962485a6a9406c5a7012139383da02";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
pub const CODEX_NATIVE_SHA256: &str =
    "544e2df9e6f09b3f1ceb0405879c83dd099ec015aeed942bb091ff0f29f60dc2";
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
pub const CODEX_NATIVE_SHA256: &str =
    "cb5e8cb8a333a408ce6adbe0d4fad1845c69772c2216af7c1f88c98a11460dc6";
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub const CODEX_NATIVE_SHA256: &str =
    "2e863156ed35ecc5253b1e2f907a9143077b9f7cb51942070c61996471ff6e04";

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub const CLAUDE_NATIVE_SHA256: &str =
    "fcbe0b8d47570c501302dd1ad31cc26ac2810f022c45fa253936a6961dee32bf";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
pub const CLAUDE_NATIVE_SHA256: &str =
    "350e657428a6d34f7cf71f6738c5ebb6a1952ccb12fc1747f64297e065b1846f";
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
pub const CLAUDE_NATIVE_SHA256: &str =
    "60e83d8db0e894d0e54413e5e7daa256d180db660f51e139a51b614fc30cf3ac";
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub const CLAUDE_NATIVE_SHA256: &str =
    "98226474f802e3094d6a86c5ade8883c16206d0fcb5c400b7401c800063e99d7";

#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "macos", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "x86_64")
)))]
compile_error!("Model Rocket has no pinned native vendor digests for this target");

/// Writes the shell-facing product contract consumed by the launcher.
///
/// # Errors
///
/// Returns the output writer's error when the contract cannot be written completely.
pub fn write_launcher_contract(
    catalogue: &ModelCatalogue,
    mut output: impl Write,
) -> io::Result<()> {
    let canonical = catalogue.canonical_route();
    for (key, value) in [
        ("default_claude_model", DEFAULT_CLAUDE_MODEL),
        ("canonical_gpt_model", canonical.claude_model.as_str()),
        ("required_claude_version", CLAUDE_CODE_VERSION_OUTPUT),
    ] {
        writeln!(output, "{key}={value}")?;
    }
    writeln!(
        output,
        "gpt_context_tokens={}",
        catalogue.minimum_context_tokens().get()
    )?;
    writeln!(output, "bridge_startup_attempts={BRIDGE_STARTUP_ATTEMPTS}")
}

/// Writes the Claude Code worktree-agent definition for the canonical GPT route.
///
/// # Errors
///
/// Returns the output writer's error when the agent definition cannot be written completely.
pub fn write_worker_config(catalogue: &ModelCatalogue, mut output: impl Write) -> io::Result<()> {
    writeln!(output, "---")?;
    writeln!(output, "name: gpt-worktree-worker")?;
    writeln!(
        output,
        "description: Use proactively for self-contained coding tasks that should run with GPT in an isolated git worktree."
    )?;
    writeln!(
        output,
        "model: {}",
        catalogue.canonical_route().claude_model.as_str()
    )?;
    writeln!(output, "isolation: worktree")?;
    writeln!(output, "---\n")?;
    writeln!(
        output,
        "Implement the delegated task in the isolated worktree."
    )?;
    writeln!(
        output,
        "Follow all repository instructions and quality gates."
    )?;
    writeln!(
        output,
        "Return a concise summary of the changes, verification evidence, and any blockers."
    )
}
