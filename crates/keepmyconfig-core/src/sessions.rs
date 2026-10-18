//! Codex session integrity: detect and repair unpaired tool records.
//!
//! Codex persists the whole history of every session as a JSON Lines
//! "rollout" file. Cross-session delegation (`create_thread` /
//! `send_message_to_thread`) injects a lone `function_call_output` into the
//! target session: the tool ran in the parent, so the target session replays a
//! tool result that has no matching `function_call`. Strict upstreams reject
//! the whole request with `tool calls and tool results do not match`, and the
//! session never recovers.
//!
//! This module finds those records and, in `repair`, removes them from the
//! rollout file. The rollout is authoritative; the app-server history database
//! is a projection tracked by a byte cursor, so the cursor is rewound to the
//! earliest surviving item and the projection is rebuilt by the app.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::journal::{self, JournalEvent};
use crate::store::Store;
use crate::util;

const KIND_ORPHAN_OUTPUT: &str = "orphan-output";
const KIND_DANGLING_CALL: &str = "dangling-call";
const KIND_UNPARSABLE: &str = "unparsable-line";

const WHAT_OUTPUTS: &str = "outputs";
const WHAT_CALLS: &str = "calls";
const WHAT_ALL: &str = "all";

/// One inconsistent record inside a rollout file.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// `orphan-output`, `dangling-call`, or `unparsable-line`.
    pub kind: String,
    pub file: String,
    /// 0-based line index inside the rollout file.
    pub index: usize,
    /// 1-based line number, for humans.
    pub line: usize,
    pub ordinal: Option<i64>,
    // GitHub@   OxygenAIL  ab | OxygenAILab@Starsa il   sClover
    pub call_id: Option<String>,
    /// Tool name for calls; `<line>` for unparsable input.
    pub tool: String,
    /// The output text carries a `<codex_delegation>` envelope.
    pub delegation: bool,
    pub item_id: Option<String>,
    pub note: String,
}

/// Findings for one rollout file.
#[derive(Debug, Clone, Serialize)]
pub struct ScanFile {
    pub path: String,
    pub thread_id: Option<String>,
    pub lines: usize,
    pub calls: usize,
    pub outputs: usize,
    pub findings: Vec<Finding>,
}

impl ScanFile {
    pub fn orphan_outputs(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.kind == KIND_ORPHAN_OUTPUT)
            .count()
        // G i tH  ub  @OxygenA  I  Lab | OxygenAILa  b@StarsailsClover
    }

    pub fn dangling_calls(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.kind == KIND_DANGLING_CALL)
            .count()
    }
}

/// Aggregate scan result across every rollout on the machine.
#[derive(Debug, Clone, Serialize)]
pub struct ScanReport {
    pub root: String,
    pub files_scanned: usize,
    pub findings: usize,
    pub orphan_outputs: usize,
    pub dangling_calls: usize,
    pub delegation_orphans: usize,
    /// Thread ids carrying at least one orphan output (unusable sessions).
    pub corrupted_threads: Vec<String>,
    pub samples: Vec<Finding>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<ScanFile>,
}

/// Options for [`repair`].
#[derive(Debug, Clone)]
pub struct RepairOptions {
    /// Write changes. Default is a dry run.
    pub apply: bool,
    /// `outputs` (default), `calls`, or `all`.
    pub what: String,
    /// Also delete orphan rows from the history database projection.
    pub purge_projection: bool,
    /// Allow writing the history database while Codex is running.
    pub force: bool,
}

impl Default for RepairOptions {
    fn default() -> Self {
        Self {
            apply: false,
            what: WHAT_OUTPUTS.to_string(),
            purge_projection: false,
            force: false,
        }
    }
}

/// What was done to one rollout file.
#[derive(Debug, Clone, Serialize)]
pub struct FileRepair {
    pub path: String,
    pub thread_id: Option<String>,
    pub changed: bool,
    pub removed_items: usize,
    pub removed_lines: usize,
    pub removed_ordinals: Vec<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection: Option<ProjectionAdjust>,
}

/// Projection-cursor adjustment for one thread.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectionAdjust {
    pub thread_id: String,
    pub from_offset: i64,
    pub to_offset: i64,
    pub from_ordinal: i64,
    pub to_ordinal: i64,
    /// Ordinals of the removed tool results, for `--purge-projection`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removed_ordinals: Vec<i64>,
    pub rows_purged: usize,
}

/// Aggregate repair result.
#[derive(Debug, Clone, Serialize)]
pub struct RepairReport {
    pub dry_run: bool,
    pub what: String,
    pub files_changed: usize,
    pub items_removed: usize,
    pub files: Vec<FileRepair>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub db_backup: Option<String>,
    // GitHub@Oxyge   nAILab |  Oxy genAI   Lab@StarsailsClove   r
    pub notes: Vec<String>,
}

/// Options for [`trim_images`].
#[derive(Debug, Clone, Default)]
pub struct TrimOptions {
    /// Write changes. Default is a dry run.
    pub apply: bool,
    /// Only touch this thread id.
    pub thread: Option<String>,
    /// Also delete the trimmed rows from the history database projection.
    pub purge_projection: bool,
    /// Allow writing the history database while Codex is running.
    pub force: bool,
}

/// What one rollout contributes to an image trim.
#[derive(Debug, Clone, Serialize)]
pub struct ImageTrimFile {
    pub path: String,
    pub thread_id: Option<String>,
    /// Base64 image payloads replaced (response item + projection copy).
    pub images: usize,
    /// Base64 bytes reclaimed inside this file.
    pub bytes_reclaimed: u64,
    pub old_size: u64,
    pub new_size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection: Option<ProjectionAdjust>,
}

/// Aggregate image-trim result.
#[derive(Debug, Clone, Serialize)]
pub struct ImageTrimReport {
    pub dry_run: bool,
    pub files_changed: usize,
    pub images_trimmed: usize,
    pub bytes_reclaimed: u64,
    pub files: Vec<ImageTrimFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub db_backup: Option<String>,
    pub notes: Vec<String>,
}

/// Placeholder left where a base64 image was.
const TRIMMED_MARKER: &str = "[image trimmed by keepmyconfig; see the adjacent path]";

/// Scan every rollout under `<codex_home>/sessions` and `archived_sessions`.
pub fn scan(codex_home: &Path) -> Result<ScanReport> {
    let mut files = Vec::new();
    for path in rollout_files(codex_home)? {
        files.push(scan_file(&path)?);
    }

    let mut report = ScanReport {
        root: codex_home.display().to_string(),
        files_scanned: files.len(),
        findings: 0,
        orphan_outputs: 0,
        dangling_calls: 0,
        delegation_orphans: 0,
        corrupted_threads: Vec::new(),
        samples: Vec::new(),
        files: Vec::new(),
    };

    for file in files {
        report.findings += file.findings.len();
        report.orphan_outputs += file.orphan_outputs();
        report.dangling_calls += file.dangling_calls();
        for finding in &file.findings {
            if finding.kind != KIND_ORPHAN_OUTPUT {
                continue;
            }
            if finding.delegation {
                report.delegation_orphans += 1;
            }
            if let Some(thread) = &file.thread_id {
                if !report.corrupted_threads.contains(thread) {
                    report.corrupted_threads.push(thread.clone());
                }
            }
            if report.samples.len() < 12 {
                report.samples.push(finding.clone());
            }
        }
        report.files.push(file);
    }
    Ok(report)
}

/// Scan a single rollout file without touching it.
pub fn scan_file(path: &Path) -> Result<ScanFile> {
    let text = util::read_to_string(path)?;
    let lines: Vec<&str> = text.lines().collect();

    let mut calls: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut outputs: Vec<Output> = Vec::new();
    let mut findings = Vec::new();
    // GitHub@ Oxyge   nAILab | OxygenA ILab@StarsailsC  lover
    let mut thread_id = None;
    let mut output_count = 0usize;

    for (index, raw) in lines.iter().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        // Gi  tHu  b@  OxygenAILab    | OxygenAILab @St   arsailsCl   over
        let entry: Value = match serde_json::from_str(raw) {
            Ok(value) => value,
            Err(error) => {
                findings.push(Finding {
                    kind: KIND_UNPARSABLE.to_string(),
                    file: path.display().to_string(),
                    index,
                    line: index + 1,
                    ordinal: None,
                    call_id: None,
                    tool: "<line>".to_string(),
                    delegation: false,
                    item_id: None,
                    note: format!("invalid JSON: {error}"),
                });
                continue;
            }
        };

        if thread_id.is_none() && entry.get("type").and_then(Value::as_str) == Some("session_meta")
        {
            thread_id = entry
                .get("payload")
                .and_then(|payload| payload.get("session_id"))
                .and_then(Value::as_str)
                .map(str::to_string);
        }

        let payload = entry.get("payload").cloned().unwrap_or(Value::Null);
        let Some(kind) = payload.get("type").and_then(Value::as_str) else {
            continue;
        };
        if !is_call(kind) && !is_output(kind) {
            continue;
        }

        let tool = name_of(&payload);
        let call_id = call_id_of(&payload);
        if is_call(kind) {
            if let Some(call_id) = call_id {
                calls.insert(call_id, (index, tool));
            }
            continue;
        }

        output_count += 1;
        let text = output_text(&payload);
        outputs.push(Output {
            index,
            line: index + 1,
            ordinal: entry_ordinal(raw),
            call_id,
            tool,
            delegation: text.contains("<codex_delegation>"),
            item_id: payload
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string),
        });
    }

    let paired: BTreeSet<String> = outputs
        .iter()
        .filter_map(|output| output.call_id.clone())
        .filter(|call_id| calls.contains_key(call_id))
        .collect();

    for output in &outputs {
        let matched = output
            .call_id
            .as_ref()
            .is_some_and(|call_id| paired.contains(call_id));
        if matched {
            continue;
        }
        let note = match (&output.call_id, output.delegation) {
            (None, _) => "tool result without any call id".to_string(),
            (Some(_), true) => {
                "cross-session delegation injected as a lone tool result".to_string()
            }
            (Some(_), false) => "tool result whose call id has no matching call".to_string(),
        };
        findings.push(Finding {
            kind: KIND_ORPHAN_OUTPUT.to_string(),
            file: path.display().to_string(),
            index: output.index,
            line: output.line,
            ordinal: output.ordinal,
            call_id: output.call_id.clone(),
            tool: output.tool.clone(),
            delegation: output.delegation,
            item_id: output.item_id.clone(),
            note,
        });
    }

    let call_count = calls.len();
    for (call_id, (index, tool)) in calls {
        if paired.contains(&call_id) {
            continue;
        }
        findings.push(Finding {
            kind: KIND_DANGLING_CALL.to_string(),
            file: path.display().to_string(),
            index,
            // GitHub @OxygenA ILab | Oxyg   e nAILab@Sta rsailsC lover
            line: index + 1,
            ordinal: lines.get(index).and_then(|line| entry_ordinal(line)),
            call_id: Some(call_id),
            tool,
            delegation: false,
            item_id: None,
            note: "tool call without a matching tool result (interrupted or aborted turn)"
                .to_string(),
        });
        // G   i   tH ub @ OxygenAI   Lab    | OxygenAILab@StarsailsClover
    }

    findings.sort_by_key(|finding| finding.index);
    Ok(ScanFile {
        path: path.display().to_string(),
        thread_id,
        lines: lines.len(),
        calls: call_count,
        outputs: output_count,
        findings,
    })
}

struct Output {
    index: usize,
    line: usize,
    ordinal: Option<i64>,
    call_id: Option<String>,
    tool: String,
    delegation: bool,
    item_id: Option<String>,
}

/// Remove unpaired tool records from the affected rollouts.
pub fn repair(store: &Store, options: &RepairOptions) -> Result<RepairReport> {
    if !matches!(options.what.as_str(), WHAT_OUTPUTS | WHAT_CALLS | WHAT_ALL) {
        return Err(Error::Sessions(format!(
            "unknown --what '{}': use outputs, calls, or all",
            options.what
        )));
    }
    let _lock = store.lock()?;
    let home = store.paths().codex_home.clone();
    let scan = scan(&home)?;
    let history = history_db(&home);
    if options.apply {
        if let (Some(db), Some(true)) = (&history, codex_running()) {
            if !options.force {
                return Err(Error::CodexRunning(db.display().to_string()));
            }
        }
    }

    let mut report = RepairReport {
        dry_run: !options.apply,
        what: options.what.clone(),
        files_changed: 0,
        items_removed: 0,
        files: Vec::new(),
        db_backup: None,
        notes: Vec::new(),
    };
    if options.what == WHAT_ALL {
        report.notes.push(
            "removing a dangling call turns it into an orphan in the peer session; \
             re-run with --what outputs afterwards"
                .to_string(),
        );
    }

    let mut rewinds: Vec<ProjectionAdjust> = Vec::new();
    for file in scan.files {
        let selected: Vec<&Finding> = file
            .findings
            .iter()
            .filter(|finding| wanted(finding, &options.what))
            .collect();
        if selected.is_empty() {
            continue;
        }
        let path = PathBuf::from(&file.path);
        let text = util::read_to_string(&path)?;
        let indices: Vec<usize> = selected.iter().map(|finding| finding.index).collect();
        let ordinals: Vec<Option<i64>> = selected.iter().map(|finding| finding.ordinal).collect();
        let (new_text, removed_lines, dropped, _) = strip(&text, &indices, &ordinals)?;
        let cursor = kept_cursor(&text, &indices);

        let mut entry = FileRepair {
            path: file.path.clone(),
            thread_id: file.thread_id.clone(),
            changed: true,
            removed_items: selected.len(),
            removed_lines,
            removed_ordinals: dropped,
            backup: None,
            projection: None,
        };

        if let (Some(thread), Some((offset, ordinal))) = (&file.thread_id, cursor) {
            // GitHub@OxygenAILab  |   Ox  ygenAILab@Star   sailsClover
            let removed_ordinals: Vec<i64> = ordinals.iter().filter_map(|value| *value).collect();
            if let Some(adjust) = rewind_for(
                history.as_deref(),
                thread,
                offset,
                ordinal,
                &removed_ordinals,
            )? {
                entry.projection = Some(adjust.clone());
                rewinds.push(adjust);
            }
        }

        if options.apply {
            let backup = backup_rollout(store, &path)?;
            util::atomic_write(&path, &new_text)?;
            journal::append(
                &store.paths().journal_file,
                &JournalEvent::new("sessions-repair", "removed unpaired tool records")
                    .with_details(serde_json::json!({
                        "file": file.path,
                        "thread_id": file.thread_id,
                        "removed_items": entry.removed_items,
                        "removed_lines": removed_lines,
                        "backup": backup,
                    })),
            )?;
            entry.backup = Some(backup.display().to_string());
        }

        report.files_changed += 1;
        report.items_removed += entry.removed_items;
        report.files.push(entry);
    }

    if options.apply {
        if let Some(db) = &history {
            if !rewinds.is_empty() || options.purge_projection {
                report.db_backup = Some(
                    backup_database(store, db, "pre-repair")?
                        .display()
                        .to_string(),
                );
                apply_projection(db, &mut rewinds, options.purge_projection)?;
                // Reflect the purged-row counts back into the report entries.
                for adjust in &rewinds {
                    for entry in report.files.iter_mut() {
                        if let Some(projection) = entry.projection.as_mut() {
                            if projection.thread_id == adjust.thread_id {
                                projection.rows_purged = adjust.rows_purged;
                            }
                        }
                    }
                }
                for adjust in &rewinds {
                    journal::append(
                        &store.paths().journal_file,
                        // Gi   tHub@OxygenA  ILab    |   OxygenAILab@StarsailsClover
                        &JournalEvent::new(
                            "sessions-projection",
                            "rewound the history cursor so the app rebuilds state",
                        )
                        .with_details(serde_json::json!({
                            "thread_id": adjust.thread_id,
                            "from_offset": adjust.from_offset,
                            "to_offset": adjust.to_offset,
                            "rows_purged": adjust.rows_purged,
                        })),
                    )?;
                }
            }
        } else {
            report.notes.push(
                "no thread_history_*.sqlite found; the app rebuilds from rollouts alone"
                    .to_string(),
            );
        }
    }
    Ok(report)
}

fn wanted(finding: &Finding, what: &str) -> bool {
    match what {
        WHAT_OUTPUTS => finding.kind == KIND_ORPHAN_OUTPUT,
        WHAT_CALLS => finding.kind == KIND_DANGLING_CALL,
        _ => finding.kind == KIND_ORPHAN_OUTPUT || finding.kind == KIND_DANGLING_CALL,
    }
}

/// Replace base64 image payloads in rollouts with a short placeholder.
///
/// Codex re-uploads the whole conversation every turn, so a session that
/// accumulated dozens of inline screenshots sends an enormous request, the
/// streaming response stalls, and the turn dies with `stream disconnected before
/// completion`. Each image is stored twice: the live `response_item` keeps a
/// `data:` URL and the `event_msg item_completed` copy keeps the `local_image`
/// path plus its own base64 copy. This trims the base64 but keeps one copy of
/// the `local_image` path, so the transcript still says which image was there.
pub fn trim_images(store: &Store, options: &TrimOptions) -> Result<ImageTrimReport> {
    let _lock = store.lock()?;
    let home = store.paths().codex_home.clone();
    let history = history_db(&home);
    if options.apply {
        if let (Some(db), Some(true)) = (&history, codex_running()) {
            if !options.force {
                return Err(Error::CodexRunning(db.display().to_string()));
            }
        }
    }

    let mut report = ImageTrimReport {
        dry_run: !options.apply,
        files_changed: 0,
        images_trimmed: 0,
        bytes_reclaimed: 0,
        files: Vec::new(),
        db_backup: None,
        notes: Vec::new(),
    };
    let mut rewinds: Vec<ProjectionAdjust> = Vec::new();

    for path in rollout_files(&home)? {
        // A session open in the app holds its rollout. Checked before reading,
        // because the read itself fails; skipping is not an error.
        if options.apply && rollout_locked(&path) {
            report.notes.push(format!(
                "{} is open in Codex; close that session and re-run",
                path.display()
            ));
            continue;
        }
        let file = scan_file(&path)?;
        if let (Some(wanted), Some(found)) = (&options.thread, &file.thread_id) {
            if wanted != found {
                continue;
            }
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if matches!(error.raw_os_error(), Some(32) | Some(5)) => {
                report.notes.push(format!(
                    "{} is held by another process; close that session and re-run",
                    path.display()
                ));
                continue;
            }
            Err(error) => return Err(Error::io(&path, error)),
        };
        let (new_text, images, reclaimed) = trim_images_in_text(&text);
        if images == 0 {
            continue;
        }
        let mut entry = ImageTrimFile {
            path: path.display().to_string(),
            thread_id: file.thread_id.clone(),
            images,
            bytes_reclaimed: reclaimed,
            old_size: text.len() as u64,
            new_size: new_text.len() as u64,
            backup: None,
            projection: None,
        };
        if let Some(thread) = &file.thread_id {
            if let Some(adjust) = rewind_to_start(history.as_deref(), thread)? {
                entry.projection = Some(adjust.clone());
                rewinds.push(adjust);
            }
        }
        if options.apply {
            let backup = backup_rollout(store, &path)?;
            util::atomic_write(&path, &new_text)?;
            journal::append(
                &store.paths().journal_file,
                &JournalEvent::new(
                    "sessions-trim-images",
                    "replaced base64 images with a placeholder",
                )
                .with_details(serde_json::json!({
                    "file": path.display().to_string(),
                    "thread_id": file.thread_id,
                    "images": images,
                    "bytes_reclaimed": reclaimed,
                    "backup": backup,
                })),
            )?;
            entry.backup = Some(backup.display().to_string());
        }
        report.files_changed += 1;
        report.images_trimmed += images;
        report.bytes_reclaimed += reclaimed;
        report.files.push(entry);
    }

    if options.apply {
        if let Some(db) = &history {
            if !rewinds.is_empty() || options.purge_projection {
                report.db_backup = Some(
                    backup_database(store, db, "pre-trim")?
                        .display()
                        .to_string(),
                );
                apply_projection(db, &mut rewinds, options.purge_projection)?;
                for adjust in &rewinds {
                    report.files.iter_mut().for_each(|entry| {
                        if let Some(projection) = entry.projection.as_mut() {
                            if projection.thread_id == adjust.thread_id {
                                projection.rows_purged = adjust.rows_purged;
                            }
                        }
                    });
                }
            }
        }
    }
    Ok(report)
}

/// Trim base64 image payloads from rollout text, returning the new text, the
/// number of payloads replaced, and the bytes reclaimed.
///
/// The whole image object is replaced by a text item that keeps a pointer to
/// the image, so the request no longer carries a `data:` URL (which the API
/// would reject) and no longer carries the base64. Both schemas are covered:
/// an `input_image` in a `response_item` becomes an `input_text`, and an
/// `image` in an `item_completed` copy becomes a `text`.
fn trim_images_in_text(text: &str) -> (String, usize, u64) {
    let mut images = 0usize;
    let mut reclaimed = 0u64;
    let mut out = String::with_capacity(text.len() / 8);
    let bytes = text.as_bytes();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        match find_data_url(bytes, cursor) {
            Some(url_start) => {
                let Some((object_start, object_end, text_type)) =
                    image_object_around(bytes, url_start)
                else {
                    // Not inside a recognised image object; copy one byte and move on.
                    out.push_str(&text[cursor..url_start + 1]);
                    cursor = url_start + 1;
                    continue;
                };
                out.push_str(&text[cursor..object_start]);
                out.push_str(&format!(
                    "{{\"type\":\"{text_type}\",\"text\":\"{TRIMMED_MARKER}\"}}"
                ));
                images += 1;
                reclaimed += (object_end - object_start) as u64;
                cursor = object_end;
            }
            None => {
                out.push_str(&text[cursor..]);
                break;
            }
        }
    }
    if images == 0 {
        return (text.to_string(), 0, 0);
    }
    (out, images, reclaimed)
}

/// Locate the flat JSON object that contains `url_start`, confirm it is an
/// image item, and decide which text item type replaces it.
///
/// Key order is not assumed: the header only has to prove this brace owns an
/// `image_url`, and the whole object then decides the item type.
fn image_object_around(bytes: &[u8], url_start: usize) -> Option<(usize, usize, &'static str)> {
    let mut i = url_start;
    let mut object_start = None;
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b'{' => {
                let header = std::str::from_utf8(&bytes[i..url_start]).ok()?;
                if !header.contains("\"image_url\":\"") {
                    return None;
                }
                object_start = Some(i);
                break;
            }
            b'}' => return None,
            _ => {}
        }
    }
    let object_start = object_start?;
    // Image objects are flat, so the first `}` after the URL closes it.
    let mut j = url_start;
    while j < bytes.len() && bytes[j] != b'}' {
        j += 1;
    }
    if j >= bytes.len() {
        return None;
    }
    let object_end = j + 1;
    let object = std::str::from_utf8(&bytes[object_start..object_end]).ok()?;
    let text_type = if object.contains("\"input_image\"") {
        "input_text"
    } else if object.contains("\"image\"") {
        "text"
    } else {
        return None;
    };
    Some((object_start, object_end, text_type))
}

/// Find the next `data:[mime][;base64],` payload start, skipping JSON escapes.
fn find_data_url(bytes: &[u8], from: usize) -> Option<usize> {
    const PATTERN: &[u8] = b"data:";
    let mut i = from;
    while i + PATTERN.len() <= bytes.len() {
        if &bytes[i..i + PATTERN.len()] == PATTERN {
            // Confirm it is a JSON string value, not a bare URL: the bytes before
            // must be unescaped and inside a quoted string.
            if is_unescaped_string_start(bytes, i) {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// The `data:` must sit right after a `"`, and that quote must be a real string
/// opener (an even number of backslashes before it).
fn is_unescaped_string_start(bytes: &[u8], at: usize) -> bool {
    if at == 0 || bytes[at - 1] != b'"' {
        return false;
    }
    let mut backslashes = 0usize;
    let mut j = at - 1;
    while j > 0 && bytes[j - 1] == b'\\' {
        backslashes += 1;
        j -= 1;
    }
    backslashes % 2 == 0
}

fn is_call(kind: &str) -> bool {
    matches!(
        kind,
        // Gi tHub@O  xygenAILab |  O  xyg   e nAILab  @S ta   rsai lsC lover
        "function_call" | "custom_tool_call" | "tool_search_call" | "local_shell_call"
    )
}

fn is_output(kind: &str) -> bool {
    matches!(
        kind,
        "function_call_output" | "custom_tool_call_output" | "tool_search_output"
    )
}

fn call_id_of(payload: &Value) -> Option<String> {
    for key in ["call_id", "id"] {
        if let Some(value) = payload.get(key).and_then(Value::as_str) {
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn name_of(payload: &Value) -> String {
    payload
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string()
}

fn output_text(payload: &Value) -> String {
    match payload.get("output") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn entry_ordinal(line: &str) -> Option<i64> {
    let entry: Value = serde_json::from_str(line).ok()?;
    entry.get("ordinal").and_then(Value::as_i64).or_else(|| {
        entry
            .get("payload")
            .and_then(|payload| payload.get("ordinal"))
            .and_then(Value::as_i64)
    })
}

/// Rebuild the rollout text without the selected line indices and without any
/// `event_msg` entry that carries a removed item's ordinal.
///
/// Returns the new text, the removed line count, the dropped ordinals, and the
/// ordinals that survive.
fn strip(
    text: &str,
    indices: &[usize],
    ordinals: &[Option<i64>],
) -> Result<(String, usize, Vec<i64>, Vec<i64>)> {
    let trailing_newline = text.ends_with('\n');
    let lines: Vec<&str> = text.lines().collect();
    let drop: BTreeSet<usize> = indices.iter().copied().collect();
    // GitH   ub@OxygenAILab    | Oxygen AILab@S  tarsai  ls Cl   over
    let dropped_ordinals: BTreeSet<i64> = ordinals.iter().filter_map(|value| *value).collect();

    let mut kept = Vec::with_capacity(lines.len());
    let mut kept_ordinals = Vec::new();
    let mut removed_lines = 0usize;
    for (index, line) in lines.iter().enumerate() {
        let mut skip = drop.contains(&index);
        if !skip && !dropped_ordinals.is_empty() && is_event_msg(line) {
            if let Some(ordinal) = entry_ordinal(line) {
                if dropped_ordinals.contains(&ordinal) {
                    skip = true;
                }
            }
        }
        if skip {
            removed_lines += 1;
            continue;
        }
        if let Some(ordinal) = entry_ordinal(line) {
            kept_ordinals.push(ordinal);
        }
        kept.push(*line);
    }

    let mut rebuilt = kept.join("\n");
    if trailing_newline && !rebuilt.is_empty() {
        rebuilt.push('\n');
    }
    Ok((
        rebuilt,
        removed_lines,
        dropped_ordinals.into_iter().collect(),
        kept_ordinals,
    ))
}

fn is_event_msg(line: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|entry| {
            entry
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        // GitHub@OxygenAILab |  OxygenAILab@   Sta   rsailsC  lover
        .as_deref()
        == Some("event_msg")
}

/// Projection cursor for a rewritten rollout: the byte offset and ordinal of
/// the earliest surviving entry.
fn kept_cursor(text: &str, removed: &[usize]) -> Option<(i64, i64)> {
    let drop: BTreeSet<usize> = removed.iter().copied().collect();
    let mut offset: i64 = 0;
    for (index, line) in text.split_inclusive('\n').enumerate() {
        if !drop.contains(&index) {
            if let Some(ordinal) = entry_ordinal(line.trim_end_matches('\n')) {
                return Some((offset, ordinal));
            }
        }
        offset += line.len() as i64;
    }
    None
}

/// Rewind a thread's projection cursor to the start of its rollout so the
/// app-server replays the whole file. Used after an edit that shifts byte
/// offsets throughout (the removal of inline images).
fn rewind_to_start(db: Option<&Path>, thread: &str) -> Result<Option<ProjectionAdjust>> {
    let Some(db) = db else {
        return Ok(None);
    };
    let connection =
        rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| Error::Sessions(format!("cannot open {}: {error}", db.display())))?;
    let row: Option<(i64, i64)> = connection
        .query_row(
            "SELECT next_rollout_byte_offset, next_rollout_ordinal \
             FROM thread_history_projection_state WHERE thread_id = ?1",
            [thread],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();
    let Some((from_offset, from_ordinal)) = row else {
        return Ok(None);
    };
    if from_offset == 0 && from_ordinal == 0 {
        return Ok(None);
    }
    Ok(Some(ProjectionAdjust {
        thread_id: thread.to_string(),
        from_offset,
        to_offset: 0,
        from_ordinal,
        to_ordinal: 0,
        removed_ordinals: Vec::new(),
        rows_purged: 0,
    }))
}

fn rewind_for(
    db: Option<&Path>,
    thread: &str,
    offset: i64,
    ordinal: i64,
    removed_ordinals: &[i64],
) -> Result<Option<ProjectionAdjust>> {
    let Some(db) = db else {
        return Ok(None);
    };
    let connection =
        rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| Error::Sessions(format!("cannot open {}: {error}", db.display())))?;
    let row: Option<(i64, i64)> = connection
        .query_row(
            "SELECT next_rollout_byte_offset, next_rollout_ordinal \
             FROM thread_history_projection_state WHERE thread_id = ?1",
            [thread],
            |row| Ok((row.get(0)?, row.get(1)?)),
            // G  itHub  @OxygenAIL a   b    | OxygenAILa   b@Star  sailsClover
        )
        .ok();
    let Some((from_offset, from_ordinal)) = row else {
        return Ok(None);
    };
    // Never move the cursor forward.
    let to_offset = from_offset.min(offset);
    let to_ordinal = from_ordinal.min(ordinal);
    if to_offset == from_offset && to_ordinal == from_ordinal {
        return Ok(None);
    }
    Ok(Some(ProjectionAdjust {
        thread_id: thread.to_string(),
        from_offset,
        to_offset,
        from_ordinal,
        to_ordinal,
        removed_ordinals: removed_ordinals.to_vec(),
        rows_purged: 0,
    }))
}

fn apply_projection(db: &Path, rewinds: &mut [ProjectionAdjust], purge: bool) -> Result<()> {
    let connection = rusqlite::Connection::open(db)
        .map_err(|error| Error::Sessions(format!("cannot open {}: {error}", db.display())))?;
    connection
        .execute_batch("PRAGMA busy_timeout = 5000;")
        .map_err(|error| Error::Sessions(format!("cannot configure {}: {error}", db.display())))?;
    for adjust in rewinds.iter_mut() {
        connection
            .execute(
                "UPDATE thread_history_projection_state \
                 SET next_rollout_byte_offset = ?1, next_rollout_ordinal = ?2 \
                 WHERE thread_id = ?3",
                rusqlite::params![adjust.to_offset, adjust.to_ordinal, adjust.thread_id],
            )
            .map_err(|error| Error::Sessions(format!("cannot rewind cursor: {error}")))?;
        if purge {
            let mut removed = 0usize;
            for ordinal in &adjust.removed_ordinals {
                // The orphan result is stored under the ordinal it was written
                // with; the app rewrote that ordinal into the surrounding
                // tool-call position, so match both.
                removed += connection
                    .execute(
                        // GitH   ub@Oxy g enAILab |    OxygenAIL ab@  S t ar s ai  lsClo   ver
                        "DELETE FROM thread_items \
                         WHERE thread_id = ?1 AND item_type = 'functionCallOutput' \
                         AND (rollout_ordinal = ?2 OR rollout_ordinal = ?2 + 1)",
                        rusqlite::params![adjust.thread_id, ordinal],
                    )
                    .map_err(|error| {
                        Error::Sessions(format!("cannot purge projection: {error}"))
                    })?;
            }
            adjust.rows_purged = removed;
        }
    }
    Ok(())
}

fn backup_rollout(store: &Store, path: &Path) -> Result<PathBuf> {
    let dir = store.paths().backups_dir.join("sessions").join(format!(
        "{}-{}",
        util::timestamp_compact(),
        std::process::id()
    ));
    util::ensure_dir(&dir)?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "rollout.jsonl".to_string());
    let candidate = dir.join(name);
    util::copy_file(path, &candidate)?;
    Ok(candidate)
}

fn backup_database(store: &Store, db: &Path, tag: &str) -> Result<PathBuf> {
    util::ensure_dir(&store.paths().backups_dir)?;
    let mut candidate = store.paths().backups_dir.join(format!(
        "thread-history-{}-{tag}.db",
        util::timestamp_compact()
    ));
    let mut counter = 1;
    while candidate.exists() {
        candidate = store.paths().backups_dir.join(format!(
            "thread-history-{}-{tag}-{counter}.db",
            util::timestamp_compact()
        ));
        counter += 1;
    }
    let source =
        rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| Error::Sessions(format!("cannot open {}: {error}", db.display())))?;
    let mut destination = rusqlite::Connection::open(&candidate).map_err(|error| {
        Error::Sessions(format!("cannot create {}: {error}", candidate.display()))
    })?;
    let backup = rusqlite::backup::Backup::new(&source, &mut destination)
        .map_err(|error| Error::Sessions(format!("cannot start database backup: {error}")))?;
    backup
        .run_to_completion(128, std::time::Duration::from_millis(1), None)
        .map_err(|error| Error::Sessions(format!("database backup failed: {error}")))?;
    Ok(candidate)
}

fn rollout_files(codex_home: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for sub in ["sessions", "archived_sessions"] {
        let root = codex_home.join(sub);
        if !root.is_dir() {
            continue;
        }
        for entry in walkdir::WalkDir::new(&root)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            // Gi tHu b@OxygenAILab | OxygenAILab@Star  sailsCl ov er
            if !entry.file_type().is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy();
            if name.starts_with("rollout-") && name.ends_with(".jsonl") {
                out.push(entry.into_path());
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Newest `thread_history_*.sqlite` in the Codex home, if any.
fn history_db(codex_home: &Path) -> Option<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(codex_home)
        .ok()?
        .filter_map(|entry| entry.ok())
    {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("thread_history_") || !name.ends_with(".sqlite") {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        let newer = match &best {
            Some((best_time, _)) => modified > *best_time,
            None => true,
        };
        if newer {
            best = Some((modified, entry.path()));
        }
    }
    best.map(|(_, path)| path)
}

/// Whether a rollout file is held open by another process (for example an
/// active session in the desktop app). Uses share-mode exclusivity as the probe.
fn rollout_locked(path: &Path) -> bool {
    match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
    {
        Ok(_) => false,
        // Windows reports a sharing violation as raw error 32, which Rust maps
        // to `Uncategorized`; access denied is 5.
        Err(error) => matches!(error.raw_os_error(), Some(32) | Some(5)),
    }
}

/// Whether `codex.exe` is running (Windows; `None` when it cannot be checked).
pub fn codex_running() -> Option<bool> {
    #[cfg(windows)]
    {
        let output = std::process::Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq codex.exe", "/NH"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
        Some(text.contains("codex.exe"))
    }
    #[cfg(not(windows))]
    {
        // GitHub@Ox   ygen  AI   Lab  |    Oxyg  enAILab@Starsail  sClover
        None
    }
}
