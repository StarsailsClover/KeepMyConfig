# KeepMyConfig — Design Notes

> Verified against CC Switch v3.20.4 (`farion1231/cc-switch`, tag `v3.20.4`, MIT)
> and a real Windows Codex home on 2026-10-02.

## 1. Problem

When CC Switch defines a third-party API provider for Codex, two independent
code paths replace user-owned configuration:

1. **Provider switch.** `ProviderService` writes the provider's stored
   `config.toml` text to `~/.codex/config.toml`. The user's *common* configuration
   (plugins, marketplaces, desktop/windows settings, hand-added MCP servers,
   trusted projects, notifications) is only merged back when the provider has
   `meta.commonConfigEnabled = true` and a non-empty `common_config_codex`
   snippet exists (`src-tauri/src/services/provider/live.rs:541-707`). Providers
   created by the UI default to `commonConfigEnabled = false`, so every switch
   silently drops those sections.
2. **MCP sync.** `sync_enabled_to_codex` rebuilds `[mcp_servers]` from CC
   Switch's own `mcp_servers` table and *removes the whole table when nothing is
   enabled there* (`src-tauri/src/mcp/codex.rs:285-350`). Hand-written MCP
   servers that were never registered in CC Switch are deleted even when the
   provider itself is untouched.

Evidence from the local machine (redacted):

| Artifact | Observation |
|---|---|
| `~/.codex/config.toml.bak-20261002-135358` | 4,265 bytes; contains `[plugins.*]`, `[marketplaces.*]`, `[desktop]`, `[windows]`, `[mcp_servers.prima-mock-api]`, extra `[projects.*]` |
| `~/.codex/config.toml` after switch | 2,373 bytes; only model routing, `node_repl`, and one project remain |
| `cc-switch.db` `providers.meta` | `{"commonConfigEnabled":false, ...}` for the third-party providers |
| `cc-switch.db` `mcp_servers` | only `ida-pro-mcp` and `node_repl`; `prima-mock-api` exists only in `config.toml` |

## 2. Goals

- Keep user-owned Codex settings, MCP registrations, skills, plugins/addons and
  their registration metadata alive across CC Switch switches and restarts.
- Repair damage automatically while the user works, with a bounded, explainable
  detection rule and a complete journal.
- Offer a root-cause integration with CC Switch: publish the protected overlay
  into `common_config_codex`, enable `commonConfigEnabled` on every Codex
  provider, and register protected MCP servers in CC Switch's `mcp_servers`
  table so its own sync keeps them.
- Never fight the provider identity CC Switch is switching to: `model`,
  `model_provider`, `model_providers`, `model_catalog_json`, and bearer tokens
<!-- GitHub@O xygenAILab | Oxyge nAIL  ab@Star   sailsClo  v  er -->
  always come from the live file.

## 3. Non-goals

- Not a credential manager. `auth.json` is backed up but never merged or
  restored automatically: CC Switch legitimately rotates it per provider.
- Not a Codex launcher or a proxy. KeepMyConfig does not sit in the request path.
- Not a CC Switch fork. The SQLite integration is opt-in, dry-run by default,
  and always preceded by a database backup.

## 4. Architecture

```
<CODEX_HOME>/
├── config.toml                     # live file owned by Codex + CC Switch
└── .keepmyconfig/
    ├── policy.toml                 # managed/ignored globs, merge mode, assets
    ├── state.json                  # baseline hash, counters, last event
    ├── baseline/config.toml        # last known good full config
    ├── overlay/config.toml         # protected projection of the baseline
    ├── journal.jsonl              # append-only audit of capture/repair events
    └── backups/
        ├── <ts>-pre-repair.toml
        ├── assets-<ts>/...         # optional file/directory backups
        └── cc-switch-<ts>.db       # database snapshots before adopt
```

**Overlay projection.** At capture time the live document is projected onto the
protected key space: `managed` patterns (provider identity) and `ignored`
patterns (app-owned churn such as `mcp_servers.node_repl`) are pruned.
<!-- Gi  t   Hub@OxygenAI   La b |    Oxygen  AILab@StarsailsClover -->
Everything else is stored as `overlay/config.toml`.

**Repair.** The overlay is merged into the live document. Missing protected
paths are restored. Conflicting protected paths follow `merge_mode`:
`overlay_wins` (default, protects user edits) or `live_wins`. The merged result
becomes the new baseline; the overlay is re-projected from it.

**Clobber detection.** A write is classified as a CC Switch clobber only when
the diff matches its fingerprint: protected paths disappeared *and* managed
provider keys changed, while few or no new user-owned paths appeared. Score
(balanced mode, threshold 4):

| Signal | Score |
|---|---|
| at least one protected path removed | +1 |
| at least one protected (non-pinned) path rewritten in place | +1 |
| at least three protected paths removed | +1 |
| at least eight protected paths removed | +1 |
| managed provider keys changed | +2 |
| no new user-owned paths | +1 (≤5 new: 0, otherwise −1) |
| file shrank by ≥30% | +1 |

`strict` lowers the threshold to 3; `off` disables automatic repair (manual
`repair` still works). Every decision is journaled with the evidence list.

## 5. CC Switch integration (`ccswitch adopt`)

Dry-run by default. `--apply` performs, inside one transaction, after backing up
the database:

1. Merge the overlay (minus `mcp_servers`) into `settings.common_config_codex`.
2. Set `providers.meta.commonConfigEnabled = true` for every `app_type='codex'`
   provider, preserving all other meta keys.
3. Upsert each protected `mcp_servers.<name>` into the `mcp_servers` table with
   `enabled_codex = 1`, merging into any existing `server_config` JSON.

The result is that CC Switch itself writes the user's common configuration on
every future switch and keeps user MCP servers during its MCP sync. `watch`
remains the safety net for older versions, for the local-proxy takeover path,
and for any write that bypasses the database.

## 6. Safety

- All writes are same-directory temp file + rename.
- A store lock (`fs2`) serializes concurrent one-shot and watch repairs.
- The live `config.toml` is backed up before every repair.
- CC Switch database writes refuse to run while `cc-switch.exe` is running
  unless `--force`, and always take a consistent SQLite backup first.
- Symlinks are never followed during asset backup/restore.
- No network access, no telemetry.

## 7. Command surface

| Command | Purpose |
|---|---|
| `init` | Create the store, capture the first baseline |
| `capture [--from FILE]` | Re-baseline after intentional edits, or recover protected keys from a backup file |
| `status [--check] [--json]` | Health, drift summary, CC Switch state; exit 2 on drift with `--check` |
| `diff` | Path-level comparison between live and baseline |
| `repair [--dry-run] [--prefer-live\|--prefer-overlay]` | Merge the overlay into the live config now |
| `watch [--once]` | Detect and repair clobbers continuously (or once) |
<!-- GitHu b @OxygenAILab |  Oxygen  AILab@S  tarsailsClov   er -->
| `backup --assets config,skills,plugins` | Versioned asset backup with excludes and size guards |
| `restore-assets --from latest` | Restore missing files/directories (opt-in overwrite) |
| `ccswitch inspect\|adopt\|backups\|restore` | Opt-in CC Switch database integration |
| `doctor` | Environment and compatibility report |

## 8. Compatibility

- Codex config format: any TOML that parses; unknown keys are preserved.
- CC Switch database: validated by table shape (`settings`, `providers`,
  `mcp_servers`) rather than by version number. `PRAGMA user_version` is
  recorded in reports. Verified with CC Switch v3.20.4 / schema version 19.
- Platforms: core logic is cross-platform; `adopt` targets the default CC Switch
  data directory (`~/.cc-switch/cc-switch.db`).


## 9. Codex session repair (`sessions`)

> Verified on a real Windows Codex home on 2026-10-10 against Codex
> `0.162.0-alpha.17.2` and the NewAPI -> codebuddy-cn upstream.

### 9.1 The defect

Codex delegates across sessions with `create_thread` / `send_message_to_thread`.
When a delegation lands in the target session, the app-server injects the tool
result as that session's first turn input:

```
TurnInputRequest { input: ResponseItem(FunctionCallOutput {
    id: None, call_id: None, name: Some("create_thread"),
    namespace: Some("codex_app"), output: ... "<codex_delegation>" ... }) }
```

The injected `function_call_output` has no matching `function_call` in that
session (the call happened in the *parent*). Codex persists it verbatim, so every
subsequent request replays an unpaired tool result. Strict upstreams reject the
whole request:

```
[400] code 11148 "tool calls and tool results do not match,
please start a new conversation and retry"
```

Codex retries the same body, the session is unusable, and the UI surfaces 502/503.
An orphan output by itself is accepted by the OpenAI Responses schema; it is the
unpaired tool record that the strict upstream (codebuddy-cn) refuses.

### 9.2 Verified footprint (this machine)

| Artifact | Observation |
|---|---|
| `~/.codex/logs_2.sqlite` | 11 `FunctionCallOutput { id: None, call_id: None }` turn injections, all `create_thread`/`send_message_to_thread` |
| `~/.codex/sessions/**` + `archived_sessions/**` | 8 rollout files with an orphan output at ordinal 7 / 13; 7 dangling calls elsewhere |
| `~/.codex/thread_history_1.sqlite` | `thread_items` carries the orphan `functionCallOutput` per target thread; `thread_history_projection_state` is a byte cursor into the rollout |
| NewAPI `logs` | request `2026...KLGYTU1` rejected with 11148 six times, 23:02:04 -> 23:05:51 |

### 9.3 Repair model

The rollout JSONL is authoritative; `thread_history_1.sqlite` is a projection
tracked by `(next_rollout_byte_offset, next_rollout_ordinal)`. Repair therefore:

1. **dry-run by default** and never touches a file without `--apply`;
2. records every finding first (`scan`);
3. backs up the rollout file into `<store>/backups/sessions/<ts>/` before writing;
4. rewrites the rollout on a same-directory temp file + rename (atomic), removing
   only the orphan item and its paired `event_msg item_completed`;
5. rewinds `thread_history_projection_state` to the smallest surviving
   `rollout_byte_offset` so the app-server replays the tail and rebuilds the
   projection;
6. optionally drops the orphan row from `thread_items` (`--purge-projection`);
7. refuses to touch the desktop database while `codex.exe` is running unless
   `--force`, and journals every path.

`scan` classifies findings:

| Kind | Meaning | Default action |
|---|---|---|
| `orphan-output` | output with no matching call | removed (`--what outputs`) |
| `dangling-call` | call with no matching output | reported only (`--what calls`) |

An orphan whose output text carries `<codex_delegation>` is tagged
`delegation` and is prioritised in the report.

### 9.4 Image bloat (`sessions trim-images`)

Codex re-uploads the whole conversation every turn. A session that accumulated
inline screenshots therefore sends an enormous request, the streaming response
stalls, and the turn dies with

```
stream disconnected before completion: Stream error: error decoding response body
```

while other sessions stay fast. Verified footprint on this machine: one session
carried a single user message of 8.39 MB holding **34 `input_image` items
(7.98 MB base64)** plus an `item_completed` copy with 25 more; its largest
rollout line was 8.00 MB against ≤1.31 MB for every other session, and its
requests ran 330k–830k input tokens at 15–149 s.

`trim-images` replaces *whole image objects* with a text item so the request
never carries a `data:` URL:

| Where | Before | After |
|---|---|---|
| `response_item` | `{"type":"input_image","image_url":"data:…","detail":"high"}` | `{"type":"input_text","text":"[image trimmed by keepmyconfig; see the adjacent path]"}` |
| `item_completed` copy | `{"type":"image","image_url":"data:…"}` | `{"type":"text","text":"[image trimmed by keepmyconfig; see the adjacent path]"}` |

The `local_image` entries that already hold a filesystem path are kept, so the
transcript still points at each image. Replacement is a byte-level string edit
over compact JSONL, so key order and every other byte are preserved; tests assert
the result is still valid JSON Lines and that the unrelated `data:` strings
inside prose are untouched. Key order is not assumed: the object is located by
its braces and confirmed by content, not by a fixed field sequence.

Because the edit shifts byte offsets throughout the file, the projection cursor
is rewound to the start of the rollout (`rewind_to_start`) instead of to a
surviving item, so the app replays the file and rebuilds its history.

A rollout that is open in the desktop app is skipped, not written under a live
session: on Windows the file is held with a share mode that blocks a second
writer, so the probe is an exclusive open and the collision surfaces as raw
error 32 (sharing violation) or 5 (access denied) rather than
`ErrorKind::PermissionDenied`. Skipped files are reported as notes.

### 9.5 SSE idle timeout on near-ceiling sessions

Codex aborts a streaming turn when no SSE event arrives for roughly 300 s
(the provider default) and logs
`stream disconnected before completion: idle timeout waiting for SSE` from
`codex_core::responses_retry`. A rollout near the context ceiling makes every
request carry hundreds of thousands of tokens, so the upstream's first token
can land after that window and the turn dies even though the request is
healthy. `model_providers.<id>.stream_idle_timeout_ms` raises the window; the
field name lives in `ModelProviderInfo` (verified by string search in the
installed `codex.exe`). A controlled run against a fake upstream that accepts
the request and then sends nothing showed the retry cadence follow the
configured value (8 000 ms produced one disconnect per 8 s, five retries
inside a minute). The observed incident (875 k tokens in context) logged the
first idle-timeout retry about 300 s after the turn started, and the CC Switch
proxy only logged the forwards, so the client-side value is the lever. The
live config, the overlay, and the CC Switch provider template carry `900000`
for `SailsAPI`; sessions that are already open may need the desktop app to be
restarted before they pick it up.

## 10. Cross-session collaboration bus (`bus`)

### 10.1 Why not the in-app message system

The session-repair defect in section 9 is caused by the message system itself:
`create_thread` / `send_message_to_thread` inject a lone `function_call_output`
into the target session and a strict upstream then rejects every turn. The safe
replacement is a plain shared directory that never enqueues anything into a
session's history.

### 10.2 Layout

Under the Cowork root (default `~/Documents/Cowork`):

```text
<channel>/
  README.md                 conventions, written once
  messages.md               append-only board, one block per message (people)
  bus/index.jsonl           one JSON record per message (authoritative, agents)
  bus/cursors/<actor>.json  last-seen message id for one actor
  threads/YYYY-MM-DD-*.md    long-form topics (convention, not managed)
```

The board keeps the pre-existing manual convention (`messages.md` +
`threads/`) working; the index is additive so tooling can track reads
reliably. Both files are plain text.

### 10.3 Semantics

- Ids are `m<seq>`; the index is append-only and authoritative.
- Posting is same-directory temp + rename. An identical body from the same
  sender returns the existing id instead of writing a duplicate.
- An actor's identity key is its name before any `(`, lowercased: `UI` and
  `UI (UI9)` are the same reader. Recipient matching uses this key, because a
  substring test would match `UI` inside `UI (UI9)`.
- `read --as ACTOR` bounds the window by that actor's cursor and advances it;
  `--peek` prints without moving it; `--unread` narrows to messages addressed
  to the actor; `--full` prints whole bodies.
- `popup` shows a Windows toast and never enqueues work into a session, so it
  cannot corrupt one.

### 10.4 Adopting an existing board

`bus adopt --channel C` backfills `bus/index.jsonl` from a hand-written
`messages.md` so an existing channel gains read tracking without changing its
convention. The board is parsed read-only (verified by hashing it before and
after); only the index and the actor cursors are written. Adopted history is
addressed to the broadcast pseudo-recipient `all`, which is never treated as an
actor, and every sender mentioned in it is seeded to the last id so a live
session does not suddenly see months of unread mail. Re-running `adopt` is
idempotent (matched on timestamp + sender + body). Cursor files favour the
display name recorded inside the JSON over the sanitized file name.

`rules [--lang zh|en]` prints the recommended AGENTS.md block verbatim, so the
same text can be dropped into every project.

### 10.5 Ops mailbox and the notify-only watcher

`bus watch --once --channel C --as ACTOR` reads the index, counts messages
addressed to `ACTOR`, and raises a desktop toast when something new arrived. It
then advances the cursor. The design rule is strict: **the watcher never
enqueues anything into a session.** That is the whole reason it is safe where
`send_message_to_thread` is not. A dry run counts without moving the cursor.

The shared channel `env-ops` is the mailbox every session can write to; the
Ops actor (`Ops`) owns it. `autostart install` always watches `env-ops=Ops` and
appends any extra `--bus CHANNEL=ACTOR` watches, so the Startup-folder loop (or
the Task Scheduler wrappers) carries the watcher alongside the config watch:

```text
keepmyconfig ... watch --once --quiet
keepmyconfig --cowork <root> bus watch --once --quiet --channel "env-ops" --as "Ops"
```

Both the Startup loop and the bus wrapper are regenerated from the same watch
list, so reinstalling is idempotent. Notification helper: a hidden PowerShell
`NotifyIcon` balloon, no lasting process.
