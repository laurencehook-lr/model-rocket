#!/usr/bin/env python3
from fake_codex_overflow_fatal import run


run(
    user_agent="fake-overflow-error-interrupted",
    thread_id="thread_error_interrupt",
    turn_id="turn_error_interrupt",
    error_message="fatal provider error after interrupt",
    terminal_status="interrupted",
)
