//! Application scheduling and retained-session limits.

use std::time::Duration;

pub const MAX_CONCURRENT_GPT_TURNS: usize = 64;
pub const TOOL_SESSION_TTL: Duration = Duration::from_secs(600);
