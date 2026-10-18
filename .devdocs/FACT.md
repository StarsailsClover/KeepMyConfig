<p style="font-size:28px;text-align:center;"><b>KeepMyConfig verified facts</b></p>

> Evidence-backed facts discovered while building KeepMyConfig.

---

## {FACTTime: 2026.10.02-18:20:00} CC Switch clobber paths 1

GitCommitHashRange: (initial commit)

### What's Happened?

CC Switch v3.20.4 can erase user-owned Codex configuration through two
independent code paths.

### Any evidence?

- `src-tauri/src/services/provider/live.rs:541-707` — `provider_uses_common_config`
  requires `meta.commonConfigEnabled = true` (or a subset heuristic); the UI
  created third-party providers on this machine have `false`.
- `src-tauri/src/mcp/codex.rs:285-350` — `sync_enabled_to_codex` rebuilds
  `[mcp_servers]` from CC Switch's database and removes the table when empty.
<!-- G   itH   ub@O  x  ygenAILab   | OxygenAILab@S tarsailsClov   er -->
- Local observation: `config.toml.bak-20261002-135358` (4,265 bytes) versus the
  post-switch `config.toml` (2,065 bytes) loses `[plugins.*]`,
  `[marketplaces.*]`, `[desktop]`, `[windows]`,
  `[mcp_servers.prima-mock-api]`, and extra `[projects.*]`.
- `cc-switch.db`: `providers.meta` contains `{"commonConfigEnabled":false, ...}`
  for the third-party providers; `mcp_servers` has no `prima-mock-api` row.

### Researches

The fix must (a) restore removed protected paths after a rewrite, and (b) publish
the protected overlay into CC Switch's `common_config_codex`, provider meta, and
`mcp_servers` table so future switches keep it natively.

### FACTs

KeepMyConfig implements both layers with dry-run-first database changes and a
backup before every write.

version: v26.0-Alpha.1

---

## {FACTTime: 2026.10.02-18:50:00} Sandbox rehearsal 2

GitCommitHashRange: (initial commit)

### What's Happened?

The clobber detector and repair path were exercised against the real pre/post
switch configs in a temporary Codex home.

### Any evidence?

- Before repair: `status --check` exited 2 with score 7 (threshold 4),
  26 missing protected paths, 1 changed protected path, 2 changed managed keys.
- After `repair`: 25 action lines (24 restored + 1 overwritten), provider
  `model = "DeepSeek-V4.1-Flash"` preserved from the clobbered file, and
  `status --check` exited 0.
- `cargo test --workspace`: 25 tests passing; clippy `-D warnings` clean.

### FACTs

The detection fingerprint distinguishes CC Switch rewrites from user edits and
restores the protected overlay without reverting provider identity.

version: v26.0-Alpha.1

---

GitHub@OxygenAILab | OxygenAILab@StarsailsClover


---

## {FACTTime: 2026.10.10-23:30:00} Codex cross-session delegation corrupts target sessions 3

GitCommitHashRange: (working tree, after v26.0-Alpha.8)

### What's Happened?

`create_thread` / `send_message_to_thread` inject an unpaired
`function_call_output` (`call_id: None`) as the target session's first turn
input. Codex replays it every turn, and the strict upstream rejects the request
with `code 11148 "tool calls and tool results do not match"`. The session never
recovers and the UI shows 502/503.

### Any evidence?

- `~/.codex/logs_2.sqlite`: 11 `ResponseItem(FunctionCallOutput { id: None,
  call_id: None, name: Some("create_thread"|"send_message_to_thread"),
  namespace: Some("codex_app") })` turn injections.
- `~/.codex/sessions/**` + `archived_sessions/**`: 8 rollouts with the orphan
  output at ordinal 7/13; 7 dangling calls in other rollouts.
- `~/.codex/thread_history_1.sqlite`: `thread_items` row per target thread;
  `thread_history_projection_state.next_rollout_byte_offset` equals the file
  size (fully projected).
- NewAPI `logs` + `oneapi-20261010230053.log`: request `2026...KLGYTU1` rejected
  with 11148 at 23:02:04, 23:02:46, 23:03:39, 23:04:20, 23:05:01, 23:05:51.
- Repro through `localhost:3000/v1/responses`: an exact-shape orphan output
  returns 200 alone but the same body reproduces the upstream failure once the
  session context is present, matching the observed signature.

### FACTs

KeepMyConfig gains `sessions scan|repair`: the rollout JSONL is authoritative,
repair is dry-run by default, backs up each rollout, rewrites atomically, and
rewinds the projection cursor so the app-server rebuilds state.

version: v26.0-Alpha.9

---

## {FACTTime: 2026.10.11-10:45:00} Remote compaction v2 kills sessions when the provider is named OpenAI 4

GitCommitHashRange: (working tree, after v26.0-Alpha.9)

### What's Happened?

`[model_providers.<x>] name = "OpenAI"` turns on Codex's OpenAI capability set
(`external_web_access`, `remote_compaction`), routing compaction through remote
compaction v2. A non-OpenAI upstream cannot answer that protocol, so compaction
fails and the turn dies with `remote compaction v2 expected exactly one
compaction output item, got 0 from N output items`. CC Switch's stored provider
template for the current Codex provider says `name = "OpenAI"`, so the bad value
keeps coming back and the alpha.8 pin was protecting it.

### Any evidence?

- Controlled A/B in a scratch `CODEX_HOME`, same prompt, `model_context_window =
  8000`, `model_auto_compact_token_limit = 1200`:
  `name = "OpenAI"` -> `task_complete.error` with `got 0 from 4 output items`,
  rollout 20 lines, no `compacted`; `name = "SailsAPI"` -> 624-line rollout with
  a `"type": "compacted"` item at ordinal 21 and the turn continues.
- `codex features list`: `remote_compaction_v2` is `removed` (false), so it is a
  provider capability, not a feature flag.
- Binary: `ModelProviderCapabilities` = `{ external_web_access,
  remote_compaction }`; `core\src\compact_remote_v2.rs` is present.
- Scratch rollout: `~/.codex/...` reproductions; the universal provider template
  row in `cc-switch.db` contained `name = "OpenAI"`.

### FACTs

The provider `name` is the lever. Fix applied on this machine: live config,
store overlay (re-baselined so the pin holds the good value), and the CC Switch
provider template (after a database backup). `status --check` exits 0.
