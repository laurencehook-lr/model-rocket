//! Resource and timing limits for Codex App Server execution.

use std::time::Duration;

pub const RPC_TIMEOUT: Duration = Duration::from_secs(20);
pub const TURN_TIMEOUT: Duration = Duration::from_secs(600);
pub const MAX_APP_SERVER_FRAME_BYTES: usize = 8 * 1024 * 1024;
pub const INITIAL_RPC_REQUEST_ID: u64 = 1;
pub const RPC_REQUEST_ID_INCREMENT: u64 = 1;
pub const MODEL_LIST_PAGE_SIZE: u64 = 100;
pub const MAX_MODEL_LIST_PAGES: usize = 256;
pub const MODEL_LIST_PAGE_DECREMENT: usize = 1;
pub const ISOLATED_HOME_MODE: u32 = 0o700;
pub const TOKEN_BOUNDARY_BACKOFF: usize = 1;
pub const TERMINATE_CHILD_ON_DROP: bool = true;
pub const CONNECTION_ALIVE_AT_START: bool = true;
pub const CONNECTION_FAILED_STATE: bool = false;
