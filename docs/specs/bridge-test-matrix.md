# Claude Code Model Router Test Matrix

| Requirement | Verification |
| ----------- | ------------ |
| REQ-001 | The launcher fixture proves explicit-path selection over a failing `PATH` shim, rejects a relative binary path and caller routing flags, and captures the launch-scoped loopback routing environment. Product acceptance repeats the settings-conflict test with the pinned official Claude Code binary. |
| REQ-002 | A fake Anthropic model list passes through, and the pinned unmodified Claude Code `/model` picker displays its Anthropic models plus the custom GPT model. |
| REQ-003 | A Claude request reaches the fake Anthropic server with its body and OAuth bearer, but without the router token or API-key header. |
| REQ-004 | App Server process tests require ChatGPT account type and the exact configured model. |
| REQ-005 | In one router process, send Claude, GPT, and Claude requests and assert each reaches only its selected adapter. |
| REQ-006, REQ-006A, REQ-006B | Tests decode exact JSON history, check developer instructions, map a structured-output schema exactly, observe an early GPT delta, validate non-stream text and tool JSON, round-trip a reserved `mcp__` tool name through a safe internal identifier, reject unregistered tool identifiers, assert exact completed-turn usage, and prove unavailable tool-boundary usage is omitted. |
| REQ-007 | An output-limit test proves truncation at a UTF-8 boundary and the `max_tokens` stop reason. |
| REQ-008 | Negative tests cover unknown GPT models, unsupported fields, invalid or duplicate Claude tool names, unregistered Codex tool identifiers, bad authentication, and unavailable dependencies. |
| REQ-009 | Authentication tests accept only the matching launch-generated base-path bearer and prove that neither it nor the retired bearer header reaches an upstream request. |
| REQ-010 | Tool tests prove same-session continuation, cross-session rejection, and bounded pending-session lifetime. |
| REQ-011 | Proxy, launcher, and child-environment tests prove the Anthropic bearer and ambient credentials cannot reach Codex. |
| REQ-012 | Source inspection and dependency review find no CA, patching, token persistence, API-key mode, or undocumented provider call. |
| REQ-013 | Configuration and HTTP tests cover loopback binding, automatic ports, redirect rejection, request and repeated-delta response limits, and secret-free logs. |
| REQ-014 | Version, frame-limit, session-limit, and real App Server capture tests prove the isolated GPT tool surface and resource bounds. |
| REQ-015 | The launcher test proves the lead session receives exact model `claude-fable-5`. |
| REQ-016 | Validate the supplied subagent frontmatter, then invoke it from the Fable lead session and confirm the GPT request uses a separate git worktree. |
| REQ-017 | The launcher test proves that Claude Code receives the exact 272,000-token custom-model context value. |
| REQ-018 | The launcher test validates owner-only permissions on the launch-scoped routing settings and proves cleanup after Claude Code exits. |
| REQ-019 | The launcher test supplies hostile proxy variables and proves that both proxy-bypass forms contain loopback hosts plus the pre-existing entries; product acceptance confirms the pinned Claude Code request reaches the router without reaching the capture proxy. |

The local code-quality gate is `just verify-full`.
The product acceptance is a separate interactive Claude Code test using fake providers first, then separately approved subscription accounts.
