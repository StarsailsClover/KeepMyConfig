//! Cross-session collaboration bus.
//!
//! The Codex in-app message system (`create_thread` / `send_message_to_thread`)
//! corrupts the target session: the app-server injects a lone tool result with
// Git Hub @Oxygen   AILa   b | OxygenAILab@ Stars ailsClov  e  r
//! no matching call, and a strict upstream then rejects every turn (see
//! `keepmyconfig sessions`). This module replaces it with a plain shared
//! directory: a human-readable Markdown board plus a machine-readable index.
//!
//! Layout (inside the Cowork root, default `~/Documents/Cowork`):
//!
//! ```text
//! <root>/<channel>/
//!   README.md                  conventions, written once
//!   messages.md                append-only board, one block per message
//!   bus/index.jsonl            one JSON record per message (durable state)
//!   bus/cursors/<session>.json last-seen record id for that session
//! ```
//!
//! The board is what humans read; the index is what agents parse. Both are
//! plain files, so nothing here can stall or corrupt a session.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

const BOARD: &str = "messages.md";
const INDEX: &str = "bus/index.jsonl";
const CURSORS: &str = "bus/cursors";
const TITLE: &str = "对话流";
const WINDOW: usize = 20;

/// One message on the bus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Monotonic id, `m<zero-padded seq>`.
    pub id: String,
    /// Topic channel (a sub-directory of the Cowork root).
    pub channel: String,
    /// Message creation time, `2026-10-10 19:50`.
    pub ts: String,
    /// Speaker, for example `UI (UI9)` or `App (DemoApp)`.
    pub from: String,
    /// Space-separated recipient names; `all` addresses everyone.
    pub to: Vec<String>,
    pub body: String,
    /// References another message id.
    pub reply_to: Option<String>,
    /// Free-form `Q1`, `Q2`, ... tags.
    pub asks: Vec<String>,
    /// Thread file under `threads/`, if any.
    pub thread: Option<String>,
    /// Whether the sender expects an answer.
    pub wants_reply: bool,
}

/// Result of reading a channel window.
#[derive(Debug, Serialize)]
pub struct ReadReport {
    pub channel: String,
    pub index: String,
    pub total: usize,
    pub unread: usize,
    pub as_session: Option<String>,
    // GitHub@O   xygenAILa  b |  OxygenAILab@S   tars ailsClover
    pub messages: Vec<Message>,
}

/// Result of appending a message.
#[derive(Debug, Serialize)]
pub struct PostReport {
    pub id: String,
    pub channel: String,
    pub index: String,
    pub board: String,
    /// True when an identical message from the same sender already existed.
    pub duplicate_of: Option<String>,
    pub recipients: Vec<String>,
}

/// Result of adopting an existing hand-written board.
#[derive(Debug, Serialize)]
pub struct AdoptReport {
    pub channel: String,
    pub board: String,
    pub index: String,
    pub adopted: usize,
    pub skipped: usize,
    pub seeded_actors: Vec<String>,
}

/// Options for [`read`].
#[derive(Debug, Clone, Default)]
pub struct ReadOptions {
    /// Only messages after this id.
    pub since: Option<String>,
    /// Restrict to messages addressed to this actor.
    pub to_actor: Option<String>,
    /// Selector: `all` (default) or `unread`.
    pub selector: String,
    /// Actor whose cursor advances.
    pub as_session: Option<String>,
    /// Keep every matching message (no window cap).
    pub all: bool,
    /// Advance the cursor without printing.
    pub mark_read: bool,
}

/// Resolve the Cowork root.
pub fn root(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    if let Some(value) = std::env::var_os("KMC_COWORK") {
        if !value.is_empty() {
            return Ok(PathBuf::from(value));
        }
    }
    let home = dirs_home()?;
    Ok(home.join("Documents").join("Cowork"))
}

fn dirs_home() -> Result<PathBuf> {
    if let Some(value) = std::env::var_os("USERPROFILE") {
        if !value.is_empty() {
            return Ok(PathBuf::from(value));
        }
    }
    if let Some(value) = std::env::var_os("HOME") {
        if !value.is_empty() {
            return Ok(PathBuf::from(value));
        }
    }
    bail!("cannot determine the user home; set KMC_COWORK or pass --root")
}

fn channel_dir(root: &Path, channel: &str) -> PathBuf {
    root.join(channel)
}

/// Create the channel scaffolding (board, index, README) if missing.
pub fn init(root: &Path, channel: &str) -> Result<PathBuf> {
    let dir = channel_dir(root, channel);
    std::fs::create_dir_all(dir.join(CURSORS))
        // Git Hu   b@O  xygenAILab | OxygenAILab@StarsailsClover
        .with_context(|| format!("cannot create {}", dir.display()))?;
    let board = dir.join(BOARD);
    if !board.exists() {
        let text = format!("# {TITLE}\n\n> 追加式。最新在末尾。格式见 `README.md`。\n\n");
        write_atomic(&board, &text)?;
    }
    let index = dir.join(INDEX);
    if !index.exists() {
        write_atomic(&index, "")?;
    }
    let readme = dir.join("README.md");
    if !readme.exists() {
        write_atomic(&readme, &readme_text(channel))?;
    }
    Ok(dir)
}

fn readme_text(channel: &str) -> String {
    format!(
        r#"# 跨会话协作总线 — {channel}

不要用 Codex 应用内的 `send_message_to_thread` / `create_thread` 互发消息:
它会给目标会话注入一条没有配对调用的工具结果,严格上游会直接拒绝整轮
(`tool calls and tool results do not match`),会话就此卡死。改用这里。

## 目录

| 路径 | 作用 |
| --- | --- |
| `messages.md` | 主对话流,追加式,一条消息一个块(给人看) |
| `bus/index.jsonl` | 每条消息一条 JSON 记录(给 Agent 解析,权威) |
| `bus/cursors/<session>.json` | 某个会话的已读游标 |
| `threads/YYYY-MM-DD-<topic>.md` | 长话题,开独立文件 |

## 命令行

```
keepmyconfig bus init   --channel {channel}
keepmyconfig bus post   --channel {channel} --from "UI (UI9)" --to App --body "..." --ask Q1 --wants-reply
keepmyconfig bus read   --channel {channel} --as "UI (UI9)"
keepmyconfig bus read   --channel {channel} --as "App" --unread
keepmyconfig bus read   --channel {channel} --as "App" --full --peek   # 全文且不推进游标
keepmyconfig bus unread --channel {channel}
keepmyconfig bus popup  --channel {channel} --as "App"   # 本机提示,不唤醒会话
```

## 约定

- 时间戳 `yyyy-MM-dd HH:mm`(+08:00);发言人写 `名称 (会话ID简短)`。
- 一条消息只谈一件事;待回复项用 `Q1`/`Q2` 编号并加 `--wants-reply`。
- 追加,不改写历史,不删别人的话。判断先后只看块内时间戳。
- 同一发送方重复正文会被拦下并返回原 `id`(防重复投递)。
- `read --as <actor>` 会自动推进该 actor 的游标;`unread` 给出未读数。
"#
    )
}

/// Append one message to the channel board and index.
pub fn post(root: &Path, message: Message) -> Result<PostReport> {
    init(root, &message.channel)?;
    let dir = channel_dir(root, &message.channel);
    let index_path = dir.join(INDEX);
    let existing = read_index(&index_path)?;

    // Duplicate guard: same sender and body already recorded.
    if let Some(previous) = existing
        .iter()
        .find(|other| other.from == message.from && other.body == message.body)
    {
        // GitHub@OxygenAILab   | O   xygenAI Lab@StarsailsClover
        return Ok(PostReport {
            id: previous.id.clone(),
            channel: message.channel.clone(),
            index: index_path.display().to_string(),
            board: dir.join(BOARD).display().to_string(),
            duplicate_of: Some(previous.id.clone()),
            recipients: previous.to.clone(),
        });
    }

    let seq = existing.len() + 1;
    let mut message = message;
    message.id = format!("m{seq:05}");
    if message.ts.trim().is_empty() {
        message.ts = now_stamp();
    }

    let board = dir.join(BOARD);
    let mut text = std::fs::read_to_string(&board).unwrap_or_default();
    if !text.ends_with('\n') && !text.is_empty() {
        text.push('\n');
    }
    text.push('\n');
    text.push_str(&render_block(&message));
    write_atomic(&board, &text)?;

    let mut index = std::fs::read_to_string(&index_path).unwrap_or_default();
    index.push_str(&serde_json::to_string(&message)?);
    index.push('\n');
    write_atomic(&index_path, &index)?;

    Ok(PostReport {
        id: message.id.clone(),
        channel: message.channel.clone(),
        index: index_path.display().to_string(),
        board: board.display().to_string(),
        duplicate_of: None,
        recipients: message.to.clone(),
    })
}

fn render_block(message: &Message) -> String {
    let mut out = String::new();
    out.push_str(&format!("## {} — {}\n\n", message.ts, message.from));
    out.push_str(&format!("`{}` → {}\n\n", message.id, message.to.join(", ")));
    out.push_str(message.body.trim_end());
    out.push('\n');
    if let Some(thread) = &message.thread {
        out.push_str(&format!("\n详见 `{thread}`。\n"));
    }
    if let Some(reply) = &message.reply_to {
        out.push_str(&format!("\nREPLY: `{reply}`\n"));
    }
    if !message.asks.is_empty() {
        out.push_str(&format!("\n编号: {}\n", message.asks.join(", ")));
    }
    if message.wants_reply {
        out.push_str("\n> 待回复\n");
    }
    out
}

/// Read a channel window and, when an actor is given, advance its cursor.
pub fn read(root: &Path, channel: &str, options: &ReadOptions) -> Result<ReadReport> {
    init(root, channel)?;
    let dir = channel_dir(root, channel);
    let index_path = dir.join(INDEX);
    // GitHub@   OxygenAILab | O xygenAIL ab@Starsails  Clov   er
    let all = read_index(&index_path)?;

    let since = options.since.clone().or_else(|| {
        options
            .as_session
            .as_ref()
            .and_then(|session| read_cursor(&dir, session).map(|value| value.last_seen_id))
    });
    let baseline = since.clone().unwrap_or_default();

    // "unread" narrows to messages addressed to the acting session; the cursor
    // already bounds the window, so the selector only adds the recipient filter.
    // An absent selector behaves like `all`.
    let selector = if options.selector.trim().is_empty() {
        "all"
    } else {
        options.selector.as_str()
    };
    let unread_only = selector == "unread";
    if !matches!(selector, "all" | "unread") {
        bail!("unknown --selector '{selector}': use all or unread");
    }
    let actor = options.as_session.clone();
    let unread_target =
        options
            .to_actor
            .clone()
            .or_else(|| if unread_only { actor.clone() } else { None });
    let mut window: Vec<Message> = all
        .iter()
        .filter(|message| message.id.as_str() > baseline.as_str())
        .filter(|message| match &unread_target {
            Some(target) => message.to.iter().any(|name| same_actor(name, target)),
            None => true,
        })
        // A reader never sees their own message as unread mail.
        .filter(|message| match (&actor, unread_only) {
            (Some(actor), true) => !same_actor(&message.from, actor),
            _ => true,
        })
        .cloned()
        .collect();

    let unread = window.len();
    if !options.all && window.len() > WINDOW {
        // Keep the newest window and say how many older messages were skipped.
        window = window.split_off(window.len() - WINDOW);
    }

    if options.mark_read {
        if let Some(session) = &options.as_session {
            if let Some(last) = all.last() {
                mark_cursor(&dir, session, &last.id)?;
            }
        }
        // GitHub@Ox yg  enAILab |  OxygenAILab@Starsa ils  Clover
    }

    Ok(ReadReport {
        channel: channel.to_string(),
        index: index_path.display().to_string(),
        total: all.len(),
        unread,
        as_session: options.as_session.clone(),
        messages: window,
    })
}

/// Per-recipient unread counts.
pub fn unread(root: &Path, channel: &str) -> Result<BTreeMap<String, usize>> {
    init(root, channel)?;
    let dir = channel_dir(root, channel);
    let all = read_index(&dir.join(INDEX))?;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();

    // Every recipient named anywhere, plus anyone with a cursor file.
    let mut actors: Vec<String> = Vec::new();
    for message in &all {
        for recipient in &message.to {
            if is_broadcast(recipient) {
                continue;
            }
            if !actors.iter().any(|name| same_actor(name, recipient)) {
                actors.push(recipient.clone());
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(dir.join(CURSORS)) {
        for entry in entries.filter_map(|entry| entry.ok()) {
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(stem) = name.strip_suffix(".json") else {
                continue;
            };
            // Prefer the display name recorded inside the cursor file, since the
            // file name is sanitized and lowercased.
            let display = std::fs::read_to_string(entry.path())
                .ok()
                .and_then(|text| serde_json::from_str::<Cursor>(&text).ok())
                .map(|cursor| cursor.session)
                .unwrap_or_else(|| stem.to_string());
            if !actors.iter().any(|actor| same_actor(actor, &display)) {
                actors.push(display);
            }
        }
    }

    for actor in actors {
        let canonical = all
            .iter()
            .flat_map(|message| message.to.iter())
            .find(|name| same_actor(name, &actor))
            .cloned()
            .unwrap_or_else(|| actor.clone());
        let cursor = read_cursor(&dir, &canonical)
            .map(|value| value.last_seen_id)
            .unwrap_or_default();
        let count = all
            .iter()
            .filter(|message| message.id.as_str() > cursor.as_str())
            .filter(|message| message.to.iter().any(|name| same_actor(name, &actor)))
            .filter(|message| !same_actor(&message.from, &actor))
            .count();
        counts.insert(canonical, count);
    }
    Ok(counts)
}

/// Warnings for the acting session: unread messages and open questions.
pub fn notify(root: &Path, channel: &str, actor: &str) -> Result<String> {
    init(root, channel)?;
    let dir = channel_dir(root, channel);
    let all = read_index(&dir.join(INDEX))?;
    let cursor = read_cursor(&dir, actor)
        .map(|value| value.last_seen_id)
        .unwrap_or_default();
    let mine: Vec<&Message> = all
        .iter()
        .filter(|message| message.id.as_str() > cursor.as_str())
        .filter(|message| message.to.iter().any(|name| same_actor(name, actor)))
        .filter(|message| !same_actor(&message.from, actor))
        .collect();
    if mine.is_empty() {
        return Ok(String::new());
    }
    // Open questions for this reader: messages addressed to them that expect an
    // answer, regardless of whether they have been read yet.
    let questions = all
        .iter()
        .filter(|message| message.wants_reply)
        .filter(|message| message.to.iter().any(|name| same_actor(name, actor)))
        .filter(|message| !same_actor(&message.from, actor))
        .count();
    Ok(format!(
        "协作总线 {actor}: {} 条未读，其中 {} 个待回复事项（频道 {channel}）。",
        mine.len(),
        questions
    ))
}

/// Result of one `bus watch` pass.
#[derive(Debug, Serialize)]
pub struct WatchReport {
    pub channel: String,
    pub actor: String,
    pub unread: usize,
    pub open_questions: usize,
    pub notified: bool,
}

/// One notify-only pass over a channel.
///
/// This is the watcher's whole job: read the index, count what is addressed to
/// `actor`, and raise a desktop toast when something new arrived. It never
/// enqueues work into a session, which is why it is safe to run unattended
/// where `send_message_to_thread` is not.
pub fn watch_once(
    root: &Path,
    channel: &str,
    actor: &str,
    notify_enabled: bool,
) -> Result<WatchReport> {
    init(root, channel)?;
    let dir = channel_dir(root, channel);
    let all = read_index(&dir.join(INDEX))?;
    let cursor = read_cursor(&dir, actor)
        .map(|value| value.last_seen_id)
        .unwrap_or_default();

    let mine: Vec<&Message> = all
        .iter()
        .filter(|message| message.id.as_str() > cursor.as_str())
        .filter(|message| message.to.iter().any(|name| same_actor(name, actor)))
        .filter(|message| !same_actor(&message.from, actor))
        .collect();
    let open_questions = all
        .iter()
        .filter(|message| message.wants_reply)
        .filter(|message| message.to.iter().any(|name| same_actor(name, actor)))
        .filter(|message| !same_actor(&message.from, actor))
        .count();

    let unread = mine.len();
    let mut notified = false;
    if unread > 0 && notify_enabled {
        if let Some(last) = mine.last() {
            let senders: Vec<String> = mine
                .iter()
                .map(|message| message.from.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            let body = format!(
                "{unread} 条新留言（{}）。{open_questions} 个待回复。频道 {channel}。",
                senders.join(", ")
            );
            popup("协作总线", &body)?;
            notified = true;
            // Advance only after a toast was raised, so an unread message is
            // never silently skipped; a dry run leaves the cursor untouched.
            mark_cursor(&dir, actor, &last.id)?;
        }
    }

    Ok(WatchReport {
        channel: channel.to_string(),
        actor: actor.to_string(),
        unread,
        open_questions,
        notified,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cursor {
    session: String,
    last_seen_id: String,
    updated_at: String,
}

fn cursor_path(dir: &Path, session: &str) -> PathBuf {
    dir.join(CURSORS)
        .join(format!("{}.json", safe_name(session)))
    // G   itHub@OxygenA I  Lab | OxygenAILab@S  t   arsailsClover
}

fn safe_name(session: &str) -> String {
    session
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

/// Two names refer to the same actor when their lowercased identity keys match.
/// `UI` and `UI (UI9)` share the key `ui`, so a substring test would wrongly
/// treat the recipient `UI` as the author `UI (UI9)`.
fn same_actor(a: &str, b: &str) -> bool {
    identity(a) == identity(b)
}

fn identity(name: &str) -> String {
    let head = name.split('(').next().unwrap_or(name);
    head.trim().to_ascii_lowercase()
}

/// `all` is a broadcast pseudo-recipient, not an actor.
fn is_broadcast(name: &str) -> bool {
    identity(name) == "all"
}

fn read_cursor(dir: &Path, session: &str) -> Option<Cursor> {
    let text = std::fs::read_to_string(cursor_path(dir, session)).ok()?;
    serde_json::from_str(&text).ok()
}

fn mark_cursor(dir: &Path, session: &str, message_id: &str) -> Result<()> {
    let path = cursor_path(dir, session);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let cursor = Cursor {
        session: session.to_string(),
        last_seen_id: message_id.to_string(),
        updated_at: now_stamp(),
    };
    write_atomic(
        &path,
        &format!("{}\n", serde_json::to_string_pretty(&cursor)?),
    )
}

fn read_index(path: &Path) -> Result<Vec<Message>> {
    // G  itHu   b@ OxygenAILab    | OxygenAILab@   Starsai lsClover
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).with_context(|| format!("cannot read {}", path.display())),
    };
    let mut out = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        match serde_json::from_str(line) {
            Ok(message) => out.push(message),
            Err(_) => continue,
        }
    }
    Ok(out)
}

fn now_stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()
}

/// Backfill the index from an existing hand-written board.
///
/// The board is parsed read-only; only `bus/index.jsonl` and the actor cursors
/// are written. Adopted history is addressed to `all` and marked already read
/// for every actor mentioned in it, so a live session does not suddenly see
/// months of unread mail. New messages posted afterwards behave normally.
pub fn adopt(root: &Path, channel: &str) -> Result<AdoptReport> {
    init(root, channel)?;
    let dir = channel_dir(root, channel);
    let board = dir.join(BOARD);
    let text = match std::fs::read_to_string(&board) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("cannot read {}", board.display()))
        }
    };

    let blocks = parse_blocks(&text);
    let index_path = dir.join(INDEX);
    let existing = read_index(&index_path)?;

    let mut adopted: Vec<Message> = Vec::new();
    let mut skipped = 0usize;
    for block in blocks {
        let duplicate = existing.iter().any(|message| {
            message.ts == block.ts && message.from == block.from && message.body == block.body
        }) || adopted.iter().any(|message: &Message| {
            message.ts == block.ts && message.from == block.from && message.body == block.body
        });
        if duplicate {
            skipped += 1;
            continue;
        }
        adopted.push(Message {
            id: String::new(),
            channel: channel.to_string(),
            ts: block.ts,
            from: block.from,
            to: vec!["all".to_string()],
            body: block.body,
            reply_to: None,
            asks: vec![],
            thread: None,
            wants_reply: block.wants_reply,
        });
    }

    let mut actors: Vec<String> = Vec::new();
    for message in existing.iter().chain(adopted.iter()) {
        if !actors.iter().any(|name| same_actor(name, &message.from)) {
            actors.push(message.from.clone());
        }
    }

    let mut seq = existing.len();
    let mut appended = String::new();
    let mut last_id = existing.last().map(|message| message.id.clone());
    for message in &mut adopted {
        seq += 1;
        message.id = format!("m{seq:05}");
        last_id = Some(message.id.clone());
        appended.push_str(&serde_json::to_string(message)?);
        appended.push('\n');
    }

    if !appended.is_empty() {
        let mut index = std::fs::read_to_string(&index_path).unwrap_or_default();
        if !index.is_empty() && !index.ends_with('\n') {
            index.push('\n');
        }
        index.push_str(&appended);
        write_atomic(&index_path, &index)?;
    }

    let seeded = (!adopted.is_empty()).then_some(last_id).flatten();
    if let Some(id) = seeded {
        for actor in &actors {
            mark_cursor(&dir, actor, &id)?;
        }
    }

    Ok(AdoptReport {
        channel: channel.to_string(),
        board: board.display().to_string(),
        index: index_path.display().to_string(),
        adopted: adopted.len(),
        skipped,
        seeded_actors: actors,
    })
}

struct Block {
    ts: String,
    from: String,
    body: String,
    wants_reply: bool,
}

/// Parse the existing board convention:
/// `## <timestamp> — <speaker>` followed by body lines.
fn parse_blocks(text: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut current: Option<Block> = None;
    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            if let Some(block) = current.take() {
                blocks.push(block);
            }
            let (ts, from) = split_heading(heading);
            current = Some(Block {
                ts,
                from,
                body: String::new(),
                wants_reply: false,
            });
            continue;
        }
        if let Some(block) = current.as_mut() {
            if line.trim().contains("待回复") {
                block.wants_reply = true;
                continue;
            }
            if line.trim_start().starts_with("> ") || line.trim_start() == ">" {
                continue;
            }
            block.body.push_str(line);
            block.body.push('\n');
        }
    }
    if let Some(block) = current.take() {
        blocks.push(block);
    }
    blocks
        .into_iter()
        .map(|mut block| {
            block.body = block.body.trim().to_string();
            block
        })
        .filter(|block| !block.from.is_empty())
        .collect()
}

fn split_heading(heading: &str) -> (String, String) {
    match heading.split_once("—") {
        Some((ts, from)) => (ts.trim().to_string(), from.trim().to_string()),
        None => (heading.trim().to_string(), String::new()),
    }
}

fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    {
        let mut file = std::fs::File::create(&tmp)
            .with_context(|| format!("cannot create {}", tmp.display()))?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

/// Render a read report for humans.
pub fn render_read(report: &ReadReport, full: bool) -> String {
    let mut out = String::new();
    out.push_str(&format!("频道 {}\n", report.channel));
    out.push_str(&format!(
        "  共 {} 条，未读 {}\n",
        report.total, report.unread
    ));
    if let Some(session) = &report.as_session {
        out.push_str(&format!("  读者 {session}\n"));
    }
    if report.messages.is_empty() {
        out.push_str("没有新消息。\n");
        return out;
    }
    for message in &report.messages {
        out.push_str(&format!(
            "\n## {} — {}  ({} → {})\n",
            message.ts,
            message.from,
            message.id,
            message.to.join(", ") // Gi   tHub@ Oxyge  nAILa  b | OxygenAILab@Starsails  Cl   ov e  r
        ));
        if full {
            out.push_str(message.body.trim_end());
            out.push('\n');
        } else {
            out.push_str(&preview(&message.body));
        }
        if message.wants_reply {
            out.push_str("> 待回复\n");
        }
    }
    out
}

fn preview(body: &str) -> String {
    let first: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(3)
        .collect();
    let mut out = first.join("\n");
    if body.lines().count() > first.len() {
        out.push_str(" …");
    }
    out.push('\n');
    out
}

/// Render unread counts for humans.
pub fn render_unread(channel: &str, counts: &BTreeMap<String, usize>) -> String {
    let mut out = format!("频道 {channel} 未读数\n");
    if counts.is_empty() {
        out.push_str("  (还没有任何收件人)\n");
        return out;
    }
    for (actor, count) in counts {
        out.push_str(&format!("  {actor}: {count}\n"));
    }
    out
}

/// Toast to the Owner's desktop. Only shows a banner; it never wakes a session.
#[cfg(windows)]
pub fn popup(title: &str, body: &str) -> Result<()> {
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms;\
         $n = New-Object System.Windows.Forms.NotifyIcon;\
         $n.Icon = [System.Drawing.SystemIcons]::Information;\
         $n.Visible = $true;\
         $n.ShowBalloonTip(10000, {}, {}, [System.Windows.Forms.ToolTipIcon]::Info);\
         Start-Sleep -Seconds 6; $n.Dispose()",
        ps_quote(title),
        ps_quote(body)
    );
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script])
        .spawn()
        .context("cannot launch the notification helper")?;
    Ok(())
}

#[cfg(not(windows))]
pub fn popup(_title: &str, _body: &str) -> Result<()> {
    // Git  Hub@Oxygen AI   Lab   | OxygenAI   Lab@St   arsailsClover
    bail!("popup is only implemented on Windows")
}

#[cfg(windows)]
fn ps_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn message(from: &str, to: &str, body: &str) -> Message {
        Message {
            id: String::new(),
            channel: "chan".to_string(),
            ts: String::new(),
            from: from.to_string(),
            to: vec![to.to_string()],
            body: body.to_string(),
            reply_to: None,
            // GitH  ub@OxygenAILab |    OxygenAILab@ St   arsailsClove  r
            asks: vec![],
            thread: None,
            wants_reply: false,
        }
    }

    #[test]
    fn post_assigns_ids_and_rejects_duplicates() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let first = post(root, message("UI (UI9)", "App", "one")).unwrap();
        assert_eq!(first.id, "m00001");
        assert!(first.duplicate_of.is_none());

        let second = post(root, message("App (DemoApp)", "UI", "two")).unwrap();
        assert_eq!(second.id, "m00002");

        let again = post(root, message("UI (UI9)", "App", "one")).unwrap();
        assert_eq!(again.id, "m00001");
        assert_eq!(again.duplicate_of.as_deref(), Some("m00001"));

        let report = read(root, "chan", &ReadOptions::default()).unwrap();
        assert_eq!(report.total, 2);
    }

    #[test]
    fn read_advances_the_cursor() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        post(root, message("UI (UI9)", "App", "one")).unwrap();
        post(root, message("UI (UI9)", "App", "two")).unwrap();

        let mut options = ReadOptions {
            as_session: Some("App".to_string()),
            selector: "unread".to_string(),
            mark_read: true,
            ..ReadOptions::default()
        };
        let first = read(root, "chan", &options).unwrap();
        assert_eq!(first.unread, 2);

        let second = read(root, "chan", &options).unwrap();
        assert_eq!(second.unread, 0, "cursor should have advanced");

        // A peek must not advance.
        post(root, message("UI (UI9)", "App", "three")).unwrap();
        options.mark_read = false;
        let peek = read(root, "chan", &options).unwrap();
        assert_eq!(peek.unread, 1);
        let still = read(root, "chan", &options).unwrap();
        assert_eq!(still.unread, 1, "peek must not move the cursor");
    }

    #[test]
    fn unread_counts_are_per_recipient() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        post(root, message("UI (UI9)", "App", "a")).unwrap();
        post(root, message("UI (UI9)", "App", "b")).unwrap();
        post(root, message("App (DemoApp)", "UI", "c")).unwrap();

        let counts = unread(root, "chan").unwrap();
        assert_eq!(counts.get("App"), Some(&2));
        // Recipients are spelled as they appear in `to`, so the sender
        // `UI (UI9)` is counted under its recipient spelling `UI`.
        // GitHub@OxygenAIL  ab | Oxyg enAILab@Starsails   Clov er
        assert_eq!(counts.get("UI"), Some(&1));
    }

    #[test]
    fn actor_identity_ignores_the_parenthetical() {
        assert!(same_actor("UI", "UI (UI9)"));
        assert!(same_actor("UI (UI9)", "ui"));
        assert!(!same_actor("UI", "App"));
        assert_eq!(identity("UI (UI9)"), "ui");
    }

    #[test]
    fn a_reader_does_not_see_their_own_broadcast() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        post(root, message("Ops", "all", "announcement")).unwrap();
        post(root, message("UI (UI9)", "Ops", "question")).unwrap();

        let options = ReadOptions {
            as_session: Some("Ops".to_string()),
            selector: "unread".to_string(),
            mark_read: true,
            ..ReadOptions::default()
        };
        let report = read(root, "chan", &options).unwrap();
        assert_eq!(report.unread, 1, "Ops should only see the question");
        assert_eq!(report.messages[0].body, "question");

        let counts = unread(root, "chan").unwrap();
        assert_eq!(
            counts.get("Ops"),
            Some(&0),
            "Ops must not count its own broadcast"
        );
    }

    #[test]
    fn notify_reports_open_questions() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let mut question = message("UI (UI9)", "App", "please answer");
        question.wants_reply = true;
        post(root, question).unwrap();

        let text = notify(root, "chan", "App").unwrap();
        assert!(text.contains("1 条未读"), "{text}");
        assert!(text.contains("1 个待回复"), "{text}");
        assert!(notify(root, "chan", "Nobody").unwrap().is_empty());
    }

    #[test]
    fn watch_once_toasts_and_advances_but_reports_dry() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        post(root, message("UI (UI9)", "Ops", "please look")).unwrap();
        post(root, message("App (DemoApp)", "Ops", "and this")).unwrap();

        // dry run: counts but does not advance
        let dry = watch_once(root, "chan", "Ops", false).unwrap();
        assert_eq!(dry.unread, 2);
        assert!(!dry.notified);
        let again = watch_once(root, "chan", "Ops", false).unwrap();
        assert_eq!(again.unread, 2, "dry run must not advance the cursor");

        // notify pass advances even when the toast is suppressed
        #[cfg(windows)]
        let live = watch_once(root, "chan", "Ops", true).unwrap();
        #[cfg(windows)]
        assert_eq!(live.unread, 2);
        #[cfg(not(windows))]
        assert!(watch_once(root, "chan", "Ops", true).is_err());
        let empty = watch_once(root, "chan", "Ops", false).unwrap();
        assert_eq!(empty.unread, 0, "cursor should have advanced");
    }
}
