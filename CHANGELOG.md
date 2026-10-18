# Changelog

All notable changes to KeepMyConfig are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to the BC version scheme (`v{Year}.{Major}-Alpha N`).

## [Unreleased]

## [v26.0-Alpha.9] — 2026-10-11

### Added

- `sessions scan [--full] [--json]`: report unpaired tool records across every
  Codex rollout under `sessions/` and `archived_sessions/`. Exits 2 when any
  orphan tool result is present. Findings are classified as `orphan-output`
  (a tool result with no matching call) or `dangling-call` (an interrupted
  call), and cross-session delegation results are tagged `delegation`.
- `sessions repair [--apply] [--what outputs|calls|all] [--purge-projection]
  [--force]`: dry-run by default; removes the selected records from each
  rollout, backs every rollout into `backups/sessions/<ts>/`, rewrites the file
  atomically, and rewinds the history-projection cursor so the app rebuilds its
  state. Every write is journaled. The history database is refused while
  `codex.exe` is running unless `--force`, and a database snapshot is taken
  before any projection change.
- `bus init|post|read|unread|popup [--cowork PATH]`: a cross-session
  collaboration bus that replaces the session-damaging in-app message system. It
  keeps the human-readable append-only Markdown board and adds a
  machine-readable `bus/index.jsonl` index plus per-actor read cursors. Posting
  is atomic and duplicate-safe; `read --as ACTOR` advances that actor's cursor;
  `popup` shows a desktop toast without waking any session.
- `bus adopt --channel CHANNEL`: backfill the index from an existing hand-written
  `messages.md` without touching the board. History is marked broadcast and
  already read so a live session is not flooded with unread mail; re-running is
  idempotent.
- `rules [--lang zh|en]`: print the recommended AGENTS.md collaboration block
  verbatim for pasting into any project.
- `bus watch [--once] [--dry-run] [--poll 60s] --channel C --as ACTOR`: a
  notify-only watcher. It toasts new mail addressed to `ACTOR` and advances that
  actor's cursor, and it never enqueues work into any session. `--dry-run`
  counts without moving the cursor.
- `autostart install` now also carries the collaboration-bus watcher: it always
  watches the shared `env-ops` channel for the `Ops` actor and accepts extra
  `--bus CHANNEL=ACTOR` watches. Both the Startup loop and the Task Scheduler
  wrapper are regenerated from the same list.
- `sessions trim-images [--apply] [--thread ID] [--purge-projection]`:
  replaces inline base64 images in rollouts with a short path-only placeholder.
  A session that accumulated dozens of screenshots re-uploads them every turn,
  which makes the request enormous and the streaming response stall with
  `stream disconnected before completion`. Dry-run by default; backs up each
  rollout, rewrites atomically, and rewinds the projection cursor to the start of
  the file so the app replays it. A rollout that is open in Codex (its file is
  held by the app) is skipped with a note instead of being written underneath a
  live session.

### Fixed

- The default policy now pins `model_provider` inside the `managed` subtree.
  A CC Switch live takeover that rewrites the active provider to `custom` is
  repaired instead of captured, so a session's provider identity survives every
  takeover. Switching providers on purpose is a two-step flow by design: run
  `keepmyconfig capture` first, then flip the provider in CC Switch.
- Pinning a whole table (for example `model_providers.SailsAPI`) makes `watch`
  restore the entire provider section when a takeover deletes it. A pinned
  leaf alone could not resurrect its missing managed parent, which left the
  live file pointing at a provider that no longer existed; the whole-subtree
  pin closes that gap.
- `classify` now scores a protected value that was rewritten in place, so a CC
  Switch rewrite that swaps a protected leaf (a provider `name`) is a clobber
  instead of a benign edit. This is the remote-compaction recurrence: the bad
  value used to be captured as the baseline.

### Verified

- Reproduced the Codex cross-session delegation defect on the real data: 11
  `FunctionCallOutput { id: None, call_id: None }` injections, 8 contaminated
  rollouts, and the NewAPI 400 (`code 11148 "tool calls and tool results do not
  match"`) at 23:02:04 -> 23:05:51.
- Real-data rehearsal: copying a contaminated rollout into a temporary Codex
  home and running `sessions repair --apply` removed exactly one line, kept the
  file valid JSON Lines, and cleared the re-scan (exit 2 -> 0).
- `cargo test --workspace` 65 tests passing; clippy `-D warnings` clean.
- Live incident 2026-10-18: a CC Switch restart clobbered the whole
  `[model_providers.SailsAPI]` table; with the subtree pin and a re-captured
  overlay, one `watch --once` cycle restored all 7 paths (identity and section
  together) and the repaired file diffed empty against the pre-clobber backup.

## [v26.0-Alpha.8] — 2026-10-06

### Added

- `pinned` policy list: exact paths that stay protected even inside a `managed`
  subtree. Traversal descends into managed tables only where a pinned
  descendant exists, a pinned leaf never resurrects a missing parent table, and
  removing a pinned path is treated as a clobber so `watch` repairs it.

### Verified

- Reproduced the Codex 0.160 remote-compaction failure in a temporary
  `CODEX_HOME`: a second turn with `model_auto_compact_token_limit = 2000`
  failed with `remote compaction v2 expected exactly one compaction output item,
  got 0 from 1 output items`.
- The same session resumed successfully after changing the third-party
  provider's `name` from `"OpenAI"` to `"SailsAPI"`, which routes compaction
  through the local summarization path instead of remote compaction v2.
- Pinned `model_providers.SailsAPI.name` survives a simulated CC Switch
  rewrite and is restored by `watch`.

## [v26.0-Alpha.7] — 2026-10-05

### Changed

- `ccswitch recover` now merges a union of every valid source instead of
  picking one config: all Codex provider configs (richest first, so the richest
  wins conflicts) plus a synthetic `CC Switch MCP registry` document built from
  `mcp_servers` rows with `enabled_codex = 1`. The report lists the merged
  sources. `--provider` restricts the provider configs to the named one and
  still adds the MCP registry.

### Verified

- Regression test extended with an `mcp_servers` row: recovery now restores
  both the provider-text entries (`prima-mock-api`, plugin, desktop) and the
  registry-only entry (`cu_bridge`).

## [v26.0-Alpha.6] — 2026-10-05

### Added

- `ccswitch recover [--provider NAME] [--db FILE] [--apply]`: when local
  backups are gone, scan the Codex provider configs stored in CC Switch, pick
  the one protecting the most user-owned paths, and merge it into the live
  config. Dry-run by default, pre-recover backup, journaled.
- Asset-coverage tracking: `backup` records the last snapshot in `state.json`;
  `status` prints it and `doctor` warns when no snapshot exists or it is older
  than 30 days.

### Fixed

- `status` no longer points a benign edit at `repair`. Drift without a clobber
  fingerprint is the user's or the Codex app's own write, and `repair` merges
  unconditionally — so the old hint invited reverting a change the user meant to
  keep. It now names `capture` first and offers `repair` as the deliberate revert;
  `doctor` reports the same distinction instead of a bare "no clobber fingerprint".
- The immediately started Startup helper now survives the installer session:
  a hidden VBS launcher is created through the WMI service
  (`Invoke-CimMethod Win32_Process Create`), so it is not a member of the
  caller's job object. The previous `cmd /C start` + `DETACHED_PROCESS` helper
  was killed when the session that installed it ended, leaving the machine
  unprotected until the next logon.

### Verified

- Synthetic CC Switch database test: a minimal live config recovers MCP,
  plugin, and desktop entries from the richest stored provider config; dry-run
  writes nothing; `--provider` selection by name works.
- Asset snapshot test: `state.json` records the snapshot files/bytes.
- On the affected machine: the WMI-launched hidden loop is alive across
  separate command sessions, and the first hardlink asset snapshot covers
  skills and plugins.

## [v26.0-Alpha.5] — 2026-10-05

### Fixed

- `autostart install` no longer fails when the host denies Task Scheduler task
  creation (observed on a non-elevated session: `schtasks /Create /SC ONLOGON`
  returned `Access is denied`). `--method auto` now falls back to a Startup
  folder loop script that runs `watch --once --quiet` every N minutes, so
  protection survives logons and app updates without administrator rights.

### Added

- `autostart install --method auto|task|startup`, plus Startup-script state in
  `autostart status` and `doctor`.

### Verified

- On the affected machine the fallback script was installed under the user's
  Startup folder, a watch helper was started immediately, and the store journal
  recorded the periodic one-shot run.

## [v26.0-Alpha.4] — 2026-10-05

### Added

- `autostart install|uninstall|status` (Windows): installs a logon daemon
  (`KeepMyConfig-Watch`) plus a periodic one-shot repair task
  (`KeepMyConfig-Check`, five-minute default) through `schtasks`, using small
  wrapper scripts next to the executable. `doctor` now reports autostart state.
  This closes the gap that let an update kill the manually started watch and
  leave the configuration unprotected.
- Codex-update-aware rewrite detection: a change of
  `mcp_servers.node_repl.env.BROWSER_USE_CODEX_APP_VERSION` combined with any
  protected-path removal is treated as an app-rewrite clobber, and the removal
  of three or more complete `mcp_servers` / `plugins` / `marketplaces` /
  `projects` entries adds clobber weight.

### Fixed

- A Codex app update that rewrites `config.toml` while preserving provider
  identity no longer scores below the clobber threshold. The observed 0.147
  rewrite (app build `26.930.31428` -> `26.930.31730`) dropped three MCP
  servers, four curated plugin entries, and reset reasoning effort while
  introducing `desktop.conversationDetailMode`; the old score was 3 against a
  threshold of 4, so `watch` would have captured the damage as a user edit.

### Verified

- Real incident recovery on 2026-10-05: `status --check` exited 2 with 12
  missing protected paths and one changed value; `repair` restored all 12 and
  rewrote `model_reasoning_effort` to the user value, leaving a pre-repair
  backup. `status --check` returned 0 afterwards.
- New regression tests: an app-update rewrite is a clobber; an app update with
  no losses is an edit; three complete entries removed is a clobber; a single
  entry removal remains an edit; the end-to-end `process()` test repairs the
  0.147-style rewrite instead of capturing it.

## [v26.0-Alpha.3] — 2026-10-03

### Fixed

- `notify` is no longer captured into the protected overlay. The Codex desktop app
  rewrites it with a version-scoped runtime path (`runtimes\cua_node\<hash>\...`), so
  any copy captured before an app update is stale. The explicit `repair` command is
  `manual = true` and therefore merges without requiring a clobber fingerprint, which
  meant `status` reporting a benign `Edit` (score 1, threshold 4) still led the user to
  a `repair` that silently rewrote `notify` back to a runtime directory that no longer
  exists — breaking the `turn-ended` Computer Use hook. `notify` now sits in the
  default `ignored` set alongside `mcp_servers.node_repl`, which shares the same
  lifecycle. Remove it from `ignored` in `policy.toml` to protect a hand-written value.

### Verified

- Recorded reproduction of the defect: baseline `notify` -> runtime
  `2134bcb1950af07e`, live `notify` -> runtime `81ea4d5168ddd0a3` (app update),
  `repair` -> `overwritten notify`, runtime `2134bcb1950af07e` restored.
- Regression test `manual_repair_keeps_an_app_updated_notify`, plus the existing
  `watch_repairs_a_cc_switch_style_clobber` now asserts app-owned churn is left alone.

## [v26.0-Alpha.2] — 2026-10-02

### Fixed

- Manifest path validation is now platform-independent: Windows drive-letter
  paths, UNC paths, POSIX absolute paths, empty segments, `.` and `..` are all
  rejected before any restore write, on every host OS. This also fixes the
  Linux CI leg of v26.0-Alpha.1 (the check previously relied on
  `std::path::Component`, which does not recognize `C:/...` on Unix).

### Verified

- `cargo test --workspace`: 26 tests passing on Windows and Linux CI.

## [v26.0-Alpha.1] — 2026-10-02

> Superseded by v26.0-Alpha.2 (Linux CI path-validation fix). The Windows
> artifact remains functionally identical for the primary platform.

### Added

- Protected overlay engine: captures a baseline of `config.toml`, projects the
  user-owned key space (`plugins`, `marketplaces`, `mcp_servers` minus
  app-managed `node_repl`, `desktop`, `windows`, `projects`, settings) and merges
  it back with `toml_edit` without touching provider identity.
- Clobber detection with an explainable score (protected removals + managed-key
  changes + file shrinkage) and `strict` / `balanced` / `off` modes.
- `watch` loop with a native filesystem watcher, debounce, poll backstop, store
  lock, pre-repair backups, state counters, and a JSONL journal.
- `capture --from FILE` recovery path for configurations already lost to a
  provider switch.
- Asset backup/restore for `skills/`, `plugins/`, `prompts/`, `rules/`,
  `AGENTS.md`, and `config.toml.bak-*`, with exclusions, size guards, hard-link
  mode, and backup-only handling for `auth.json`.
- Opt-in CC Switch integration: `ccswitch inspect|adopt|backups|restore`.
  Adopt merges the overlay into `common_config_codex`, enables
  `commonConfigEnabled` on every Codex provider, and upserts protected MCP
  servers into the `mcp_servers` table — dry-run by default, one transaction,
  database backup first.
- Bilingual README and usage documentation (English / 简体中文).
- CI workflow (Windows + Linux): fmt, clippy `-D warnings`, full test suite.

### Verified

- CC Switch v3.20.4 source analysis (provider switch and MCP sync paths) recorded
  in `.devdocs/DESIGN.md`.
- Real-data sandbox rehearsal on Windows: 4,265-byte pre-switch config vs
  2,065-byte post-switch file → detection score 7 (threshold 4), 26 misplaced
  protected paths restored, provider model preserved, `status --check` back to 0.
- `cargo test --workspace`: 26 tests (14 unit + 9 workflow + 3 CLI) passing;
<!-- G   itHub@OxygenA ILab | OxygenAILa   b@St  arsail  sCl  over -->
  `cargo clippy --workspace --all-targets -- -D warnings` clean.

### Security

- No network access or telemetry.
- `auth.json` is never merged or restored automatically.
- CC Switch database writes require `--apply`, take a consistent SQLite backup,
  and refuse to run while CC Switch is open unless `--force` is passed.
