# Claude Code Model Router Test Matrix

| Requirement | Verification |
| ----------- | ------------ |
| REQ-001 | Start the pinned Claude Code binary through the launcher and capture a request at the loopback router. |
| REQ-002 | A fake Anthropic model list passes through, and the pinned unmodified Claude Code `/model` picker displays its Anthropic models plus the custom GPT model. |
| REQ-003 | A Claude request reaches the fake Anthropic server with its body and OAuth bearer, but without the router token or API-key header. |
| REQ-004 | App Server process tests require ChatGPT account type and the exact configured model. |
| REQ-005 | In one router process, send Claude, GPT, and Claude requests and assert each reaches only its selected adapter. |
| REQ-006, REQ-006A | Tests decode exact JSON history, check developer instructions, map a structured-output schema exactly, observe an early GPT delta, validate non-stream text and tool JSON, assert exact completed-turn usage, and prove unavailable tool-boundary usage is omitted. |
| REQ-007 | An output-limit test proves truncation at a UTF-8 boundary and the `max_tokens` stop reason. |
| REQ-008 | Negative tests cover unknown GPT models, unsupported fields, bad authentication, and unavailable dependencies. |
| REQ-009 | Authentication tests accept only the matching launch-generated `x-model-rocket-token`. |
| REQ-010 | Tool tests prove same-session continuation, cross-session rejection, and bounded pending-session lifetime. |
| REQ-011 | Proxy, launcher, and child-environment tests prove the Anthropic bearer and ambient credentials cannot reach Codex. |
| REQ-012 | Source inspection and dependency review find no CA, patching, token persistence, API-key mode, or undocumented provider call. |
| REQ-013 | Configuration and HTTP tests cover loopback binding, automatic ports, redirect rejection, request and repeated-delta response limits, and secret-free logs. |
| REQ-014 | Version, frame-limit, session-limit, and real App Server capture tests prove the isolated GPT tool surface and resource bounds. |
| REQ-015 | The launcher test proves the lead session receives exact model `claude-fable-5`. |
| REQ-016 | Validate the supplied subagent frontmatter, then invoke it from the Fable lead session and confirm the GPT request uses a separate git worktree. |

The local code-quality gate is `just verify-full`.
The product acceptance is a separate interactive Claude Code test using fake providers first, then separately approved subscription accounts.
