#!/usr/bin/env python3
from fake_codex_overflow_fatal import run


run(
    user_agent="fake-overflow-failed",
    thread_id="thread_failed",
    turn_id="turn_failed",
    error_message="provider failed after interrupt",
    terminal_status="failed",
)
