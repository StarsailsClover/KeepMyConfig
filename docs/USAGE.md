# KeepMyConfig usage reference

> GitHub@OxygenAILab | OxygenAILab@StarsailsClover

## 1. Layout

```
<CODEX_HOME>/                      # default ~/.codex (or $CODEX_HOME)
├── config.toml
└── .keepmyconfig/
    ├── policy.toml                # managed/ignored globs, merge mode, assets
    ├── state.json                 # baseline hash, counters, last event
    ├── baseline/config.toml       # last known good full config
    ├── overlay/config.toml        # protected projection of the baseline
    ├── journal.jsonl              # append-only audit trail
    └── backups/
        ├── <ts>-pre-repair.toml
        ├── assets-<ts>/
        └── cc-switch-<ts>.db
```

Global options: `--codex-home PATH`, `--store PATH`, `--json`.
Environment overrides: `CODEX_HOME`, `KMC_CODEX_HOME`, `KMC_STORE`,
`KMC_CCSWITCH_DB` (custom CC Switch database path).

## 2. Policy

`policy.toml` is created by `init` and can be edited:

```toml
schema = 1
merge_mode = "overlay_wins"   # or "live_wins"
detection = "balanced"        # "strict", "balanced", "off"

managed = [                   # provider identity: never protected
  "model",
  "model_provider",
  "model_catalog_json",
  "base_url",
  "wire_api",
  "model_providers",
  "experimental_bearer_token",
  "web_search",
]

ignored = [                   # app-owned churn: neither protected nor restored
  "mcp_servers.node_repl",
  "notify",
]

pinned = [                    # force these paths even inside a managed subtree
  # "model_providers.SailsAPI.name",
]

[assets]
dirs = ["skills", "plugins", "prompts", "rules"]
files = ["config.toml", "AGENTS.md", "requirements.toml"]
globs = ["config.toml.bak-*"]
backup_only = ["auth.json"]
exclude = ["**/node_modules/**", "**/.git/**", "**/target/**", "**/__pycache__/**", "**/*.zip", "plugins/cache/**"]
max_file_bytes = 52428800
```

- Patterns are dotted TOML paths. A pattern matches a path **or any ancestor**,
  so `model_providers` covers the whole subtree and `mcp_servers.node_repl`
  covers its nested `env`.
- `mcp_servers.node_repl` and `notify` are rewritten by the Codex desktop app with a
  version-scoped runtime path (`runtimes\cua_node\<hash>\...`). An overlay copy captured
  before an app update is stale, and an explicit `repair` merges unconditionally, so
  protecting them would restore a path that no longer exists. Remove a key from
  `ignored` if you maintain it by hand and want it protected.
<!-- GitHub@OxygenAIL   ab | Oxyge nAILab@Star sa  ilsC  lover -->
- Keys containing dots or backslashes are quoted in the canonical path, for
  example `projects."c:\\work\\demo".trust_level`; use `projects.*` to match.
- `merge_mode = "overlay_wins"` restores your value when a protected path
<!-- GitHub@OxygenAIL ab | Oxyge  nAILab@StarsailsClover -->
  conflicts. Use `live_wins` if you prefer newer Codex writes to survive.
- `detection = "off"` disables automatic repair in `watch`; explicit `repair`
  still works.
- `pinned` entries win over `managed` for exactly those paths, which is useful
  for provider fields that must stay fixed (for example a third-party
  provider's `name`). A pinned leaf is only applied when its parent table
  exists, and removing one is treated as a clobber.

## 3. Daily workflow

### First run

```powershell
keepmyconfig init
keepmyconfig status
keepmyconfig watch           # keep this running
```

Register `watch` at logon (no admin required): press `Win+R`, run
`shell:startup`, and create `KeepMyConfig.cmd`:

```bat
@echo off
"C:\path\to\keepmyconfig.exe" watch --quiet
```

For crash recovery, use Task Scheduler with "At log on" and restart-on-failure
instead of the Startup folder.

### After changing your configuration on purpose

Codex and manual edits are captured automatically while `watch` runs. If you
edited with `watch` stopped, either start `watch` once or run:

```powershell
keepmyconfig capture
```

### After a CC Switch provider switch

With `watch` running, repair is automatic. To check:

```powershell
keepmyconfig status --check   # exit 2 when protected drift exists
keepmyconfig repair --dry-run # show what would be restored
keepmyconfig repair
```

### Surviving reboots and Codex updates (Windows)

```powershell
keepmyconfig autostart install            # logon daemon + check every 5 minutes
keepmyconfig autostart install --interval-minutes 10
keepmyconfig autostart install --method auto|task|startup
keepmyconfig autostart install --no-daemon
keepmyconfig autostart status
keepmyconfig autostart uninstall
```

`install` writes two wrapper scripts next to `keepmyconfig.exe`, registers
`KeepMyConfig-Watch` (runs `watch` at logon) and `KeepMyConfig-Check` (runs
`watch --once --quiet` on a schedule) with the current-user Task Scheduler, and
starts both once immediately. The periodic task keeps repairing even if the
daemon is killed by an app update or a reboot. When the host denies Task
Scheduler creation (common for non-elevated sessions), `auto` falls back to a
Startup-folder loop that runs a one-shot repair every N minutes; use
`--method startup` to select that path directly.

### Recovering from an existing loss

```powershell
keepmyconfig init
keepmyconfig capture --from "$HOME\.codex\config.toml.bak-20261002-135358"
keepmyconfig repair
```

The `--from` file is projected through the same policy, so only user-owned keys
are recovered; provider identity still comes from the live file.

## 4. Asset backups

```powershell
keepmyconfig backup                          # config.toml + config.toml.bak-* + auth.json
keepmyconfig backup --assets config,skills  # add skills/
keepmyconfig backup --assets all            # skills, plugins, prompts, rules
keepmyconfig backup --assets all --link     # hard links (fast, lower disk use)
keepmyconfig backup --assets plugins --include_cache
```

Restore only fills in what is missing by default:

```powershell
keepmyconfig restore-assets                 # newest asset backup
keepmyconfig restore-assets --dry-run
keepmyconfig restore-assets --overwrite
keepmyconfig restore-assets --include-backup-only   # restores auth.json too
```

Excluded by default: `node_modules`, `.git`, `target`, `__pycache__`, zip files,
and `plugins/cache/**`. Files above `max_file_bytes` (50 MiB) are skipped and
counted in the report.

`status` prints the last snapshot, and `doctor` warns when no snapshot exists
or the newest one is older than 30 days. A hardlink snapshot is cheap on the
same volume:

```powershell
keepmyconfig backup --assets all --link
```

## 5. CC Switch integration

```powershell
keepmyconfig ccswitch inspect
keepmyconfig ccswitch adopt            # dry run
keepmyconfig ccswitch adopt --apply    # requires CC Switch to be closed
keepmyconfig ccswitch adopt --apply --force   # allow while running (not recommended)
keepmyconfig ccswitch adopt --no-mcp   # do not touch the mcp_servers table
keepmyconfig ccswitch backups
keepmyconfig ccswitch restore --backup "$HOME\.codex\.keepmyconfig\backups\cc-switch-20261002-190000.db"
```

Adopt validates the database shape (`settings`, `providers`, `mcp_servers`)
instead of a version number, records `PRAGMA user_version`, and writes
everything in one transaction after taking a consistent SQLite backup.

When both the store baseline and Codex's own backups are gone, recover from the
provider configs CC Switch keeps:

```powershell
keepmyconfig ccswitch recover                 # preview, picks the richest config
keepmyconfig ccswitch recover --provider "Sails API"
keepmyconfig ccswitch recover --apply
```

The command projects every stored source through the protection policy: all
Codex provider configs (richest first, current provider wins ties) plus a
synthetic document built from `mcp_servers` rows with `enabled_codex = 1`.
It merges the union with `overlay_wins` and takes a `pre-recover` backup before
writing. Pass `--provider NAME` to use only that provider's config (the MCP
registry is still added).

## 6. Detection scoring

| Signal | Score |
<!-- GitHub @Oxyg enAILab |   OxygenAILab@StarsailsClo   v  er -->
|---|---|
| at least 1 protected path removed | +1 |
| at least 1 protected path rewritten in place | +1 |
| at least 3 protected paths removed | +1 |
| at least 8 protected paths removed | +1 |
| managed provider keys changed | +2 |
| no new user-owned paths | +1 (1–5 new: 0, more: −1) |
| file shrank by ≥30% | +1 |
| three or more complete MCP/plugin/marketplace/project entries removed | +2 |
| `BROWSER_USE_CODEX_APP_VERSION` changed **and** protected paths were removed | forced clobber |

`balanced` repairs at ≥4, `strict` at ≥3, `off` never auto-repairs.

### Session repair

Codex delegates across sessions with `create_thread` / `send_message_to_thread`.
The app-server injects the tool result into the target session as its first turn
input, but the matching tool call lives in the *parent* session. The persisted
rollout therefore replays a tool result with no matching call, and a strict
upstream rejects the whole request with `code 11148 "tool calls and tool results
do not match"`. The session never recovers, and the UI shows 502/503.

```
keepmyconfig sessions scan            # report; exit 2 when a session is damaged
keepmyconfig sessions scan --full      # every finding, not just samples
keepmyconfig sessions repair           # dry run
keepmyconfig sessions repair --apply   # remove the unpaired records
keepmyconfig sessions repair --apply --purge-projection
keepmyconfig sessions repair --apply --what calls   # interrupted calls instead
keepmyconfig sessions trim-images                   # dry run: replace inline base64 images
keepmyconfig sessions trim-images --apply           # a session bloated by screenshots
```

`scan` classifies each finding:

| Kind | Meaning | Default action |
|---|---|---|
| `orphan-output` | tool result with no matching call | removed by `--what outputs` |
| `dangling-call` | tool call with no result (aborted turn) | reported; removed by `--what calls` |

The rollout JSONL is authoritative. `repair` backs each rollout into
`backups/sessions/<timestamp>/`, rewrites it atomically, and rewinds
`thread_history_projection_state` so the app replays the tail and rebuilds its
projection. The history database is written only after a snapshot and is refused
while `codex.exe` runs unless `--force`. Cross-session delegation results are
tagged `delegation`; prefer keeping them out of the message system entirely.

### Cross-session collaboration bus

The Codex in-app message system is the very thing that damages sessions, so
`keepmyconfig bus` provides a safe replacement: a plain shared directory that no
agent tool touches the session history to use. It keeps the human-readable
Markdown board and adds a machine-readable index for reliable read tracking.

```
keepmyconfig bus init   --channel demo-ui
keepmyconfig bus post   --channel demo-ui --from "UI (UI9)" --to App \
                        --body "..." --ask Q1 --wants-reply
keepmyconfig bus read   --channel demo-ui --as "App"            # unread window, advances the cursor
keepmyconfig bus read   --channel demo-ui --as "App" --full --peek   # full bodies, no cursor move
keepmyconfig bus unread --channel demo-ui                        # per-recipient counts
keepmyconfig bus popup  --channel demo-ui --as "App"             # desktop toast; does not wake the session
keepmyconfig bus adopt  --channel demo-ui                        # index an existing hand-written board
keepmyconfig rules                                                # print the AGENTS.md block to paste
keepmyconfig bus watch  --channel env-ops --as "Ops" --once       # notify-only watcher (toast, never wakes)
```

Layout under the Cowork root (`--cowork`, `KMC_COWORK`, or `~/Documents/Cowork`):

| Path | Purpose |
|---|---|
| `<channel>/messages.md` | append-only board, one block per message (for people) |
| `<channel>/bus/index.jsonl` | one JSON record per message (authoritative, for agents) |
| `<channel>/bus/cursors/<actor>.json` | that actor's last-seen id |

Appending is atomic; an identical body from the same sender is detected and the
original id is returned instead of writing a duplicate. An actor's identity key
is its name before any parenthesis, lowercased, so `UI` and `UI (UI9)` are the
same reader. `popup` is a one-way desktop notification: it never enqueues work
into a session, which is why it is safe where `send_message_to_thread` is not.

## 7. Exit codes and JSON

| Code | Meaning |
|---|---|
| 0 | success / healthy |
| 1 | error (including `doctor` structural failures) |
| 2 | `--check` found protected drift or a clobber fingerprint |

Every report command supports `--json`; `journal.jsonl` is JSON Lines.

## 8. Troubleshooting

| Symptom | Fix |
|---|---|
| `store is not initialized` | `keepmyconfig init` |
| `another process holds .../.lock` | close the other watch/one-shot process |
| `cannot parse TOML` | the file was being written; `watch` retries, or run `keepmyconfig status` after the writer finishes |
| Repair restores an old value you no longer want | run `keepmyconfig capture` to re-baseline, or edit `policy.toml` |
| CC Switch keeps clobbering | `keepmyconfig ccswitch adopt --apply` |
| `auth.json` looks stale | it is intentionally never restored; sign in again or use `restore-assets --include-backup-only` deliberately |
| Codex desktop app fell back to the old interface and lost its translations | The shell bundles its UI over the network at start-up and falls back to the built-in classic interface whenever those fetches fail. Restore a working egress, then fully quit and reopen the app; no sign-in change is needed. A bare `curl https://chatgpt.com` returning `403` is a Cloudflare bot-block and is not a reliable signal; check `https://github.com` instead. |
| The desktop interface regressed again after a proxy or node change | Switch the Clash group (`GLOBAL`, or your subscription group) to a reachable region. Clash for Windows persists the pick in `profiles/list.yml` and `cache.db`, so it survives restarts. There is no supported setting that pins the shell version. |
| A session fails every turn with `tool calls and tool results do not match` | `keepmyconfig sessions scan` to confirm, then `sessions repair --apply`. The cause is a cross-session delegation result; see section 6. |
| One session stalls with `stream disconnected before completion` / `error decoding response body` while others are fast | It has huge inline images. `keepmyconfig sessions trim-images` reports the payloads and `--apply` replaces each with a path-only placeholder. |

## 9. Security notes

- No network calls; no telemetry.
- `auth.json` is excluded from merge/restore by default because CC Switch
  legitimately rotates it per provider.
<!-- GitHub@   OxygenA  ILab |     Oxy  genAILab@StarsailsCl  over -->
- CC Switch database writes require `--apply`, take a backup, and refuse while
  the application is running unless forced.

GitHub@OxygenAILab | OxygenAILab@StarsailsClover
