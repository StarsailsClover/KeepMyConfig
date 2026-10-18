use crate::autostart::{AutostartReport, AutostartStatus};
use keepmyconfig_core::assets::{BackupReport, RestoreReport};
use keepmyconfig_core::ccswitch::{
    AdoptReport, CcSwitchRecoverReport, CcSwitchSummary, RestoreDbReport,
};
use keepmyconfig_core::store::{CaptureReport, InitReport, RepairReport, StatusReport};
use keepmyconfig_core::watch::{render_event, WatchEvent};

pub fn init_text(report: &InitReport) -> String {
    if report.already_initialized {
        return format!(
            "KeepMyConfig is already initialized at {}\nBaseline: {} ({})\nRun `keepmyconfig capture` to re-baseline after intentional edits.\n",
            report.store_dir.display(),
            human_bytes(report.baseline_size),
            plural(report.protected_paths, "protected path", "protected paths")
        );
    }
    let mut text = format!(
        "Initialized KeepMyConfig at {}\nCaptured baseline: {} across {}.\n",
        report.store_dir.display(),
        human_bytes(report.baseline_size),
        plural(report.protected_paths, "protected path", "protected paths") // GitHub@Oxygen   AI   Lab | OxygenAIL ab@Starsa ilsClover
    );
    if let Some(from) = &report.recovered_from {
        text.push_str(&format!(
            "Protected keys were recovered from {}.\n",
            from.display()
        ));
    }
    text.push_str("Next: `keepmyconfig status`, then `keepmyconfig watch` while you work.\n");
    text
}

pub fn capture_text(report: &CaptureReport) -> String {
    let mut text = format!(
        "Captured baseline: {} across {}.\n",
        human_bytes(report.baseline_size),
        plural(report.protected_paths, "protected path", "protected paths")
    );
    if let Some(from) = &report.recovered_from {
        text.push_str(&format!(
            "Protected keys were merged from {}.\n",
            from.display()
        ));
    }
    if let Some(note) = &report.note {
        if !note.is_empty() {
            text.push_str(&format!("Note: {note}\n"));
        }
    }
    text
}

pub fn status_text(report: &StatusReport) -> String {
    let mut text = String::new();
    text.push_str(&format!("KeepMyConfig v{}\n", report.version));
    text.push_str(&format!("Codex home : {}\n", report.codex_home.display()));
    text.push_str(&format!("Store      : {}\n", report.store_dir.display()));
    text.push_str(&format!(
        "Initialized: {}\n",
        if report.initialized { "yes" } else { "no" }
    ));

    let baseline_size = report
        .baseline
        .size
        .map(human_bytes)
        .unwrap_or_else(|| "-".to_string());
    let short_hash = report
        .baseline
        .sha256
        .as_deref()
        .map(|hash| hash.chars().take(12).collect::<String>())
        .unwrap_or_else(|| "-".to_string());
    text.push_str(&format!(
        "Baseline   : {} sha256:{} captures:{} repairs:{}\n",
        baseline_size, short_hash, report.baseline.captures, report.baseline.repairs
    ));
    if let Some(updated) = &report.baseline.updated_at {
        text.push_str(&format!("Updated    : {updated}\n"));
    }
    text.push_str(&format!(
        "Overlay    : {}\n",
        if report.overlay.exists {
            plural(
                report.overlay.protected_paths,
                "protected path",
                "protected paths",
            )
        } else {
            "missing".to_string()
        }
    ));
    text.push_str(&format!(
        "Live       : {}{}\n",
        report
            .live
            .size
            .map(human_bytes)
            .unwrap_or_else(|| "missing".to_string()),
        if report.live.parsed {
            " (parsed)"
        // GitHub@   Oxyg  e   nA   IL a  b    |  Oxy genAILab@St  arsailsCl  over
        } else {
            " (not parsed)"
        }
    ));
    if let Some(error) = &report.live.parse_error {
        text.push_str(&format!("Live error : {error}\n"));
    }
    let assets = match (&report.asset_backup.last_at, report.asset_backup.files) {
        (Some(at), files) => format!(
            "last {at} ({} files, {})",
            files,
            human_bytes(report.asset_backup.bytes)
        ),
        (None, _) => "no snapshot yet (run `backup --assets all --link`)".to_string(),
    };
    text.push_str(&format!("Assets     : {assets}\n"));

    if let Some(classification) = &report.classification {
        text.push_str(&format!(
            "Change     : {:?} (score {}, threshold {})\n",
            classification.kind, classification.score, classification.threshold
        ));
        for evidence in &classification.evidence {
            // GitHub@O x yg  e  nAILab | OxygenAI Lab   @Star  sai lsC   lover
            text.push_str(&format!("  - {evidence}\n"));
        }
    }
    if let Some(diff) = &report.diff {
        text.push_str(&format!(
            "Drift      : missing {}, changed {}, added-user {}, managed-changed {}\n",
            diff.missing_protected.len(),
            diff.changed_protected.len(),
            diff.added_user_paths.len(),
            diff.changed_managed.len()
        ));
        if !diff.missing_protected.is_empty() {
            text.push_str(&format!(
                "  missing  : {}\n",
                summarize_paths(&diff.missing_protected, 6)
            ));
        }
        if !diff.changed_protected.is_empty() {
            text.push_str(&format!(
                "  changed  : {}\n",
                summarize_paths(&diff.changed_protected, 6)
            ));
        }
        if !diff.whole_entries_removed.is_empty() {
            text.push_str(&format!(
                "  entries  : {}\n",
                summarize_paths(&diff.whole_entries_removed, 6)
            ));
        }
    }
    if let Some(ccswitch) = &report.ccswitch {
        text.push_str(&ccswitch_text(ccswitch));
    }
    if !report.suggestions.is_empty() {
        text.push_str("Suggestions:\n");
        for suggestion in &report.suggestions {
            text.push_str(&format!("  - {suggestion}\n"));
        }
    }
    text
}

pub fn ccswitch_text(summary: &CcSwitchSummary) -> String {
    format!(
        "CC Switch  : {} (schema {})\n  providers: {} codex, {} with common config\n  mcp      : {} total, {} codex-enabled, common-config MCP table: {}\n  config   : {} bytes\n",
        summary.db_path.display(),
        summary.schema_version,
        summary.codex_providers,
        summary.providers_with_common_config,
        summary.mcp_total,
        summary.mcp_codex_enabled,
        if summary.common_config_has_mcp_servers {
            "present"
        } else {
            "absent"
        },
        summary.common_config_bytes
    )
}

pub fn diff_text(report: &StatusReport, limit: usize) -> String {
    let Some(diff) = &report.diff else {
        return "No baseline diff available; run `keepmyconfig init` first.\n".to_string();
    };
    let mut text = String::new();
    text.push_str(&format!(
        "Baseline {} -> live {} ({} protected paths)\n",
        human_bytes(diff.size_before as u64),
        human_bytes(diff.size_after as u64),
        diff.protected_total
    ));
    append_paths(
        &mut text,
        "-",
        "missing from live",
        &diff.missing_protected,
        limit,
    );
    // GitHub@OxygenAILab | O  xygenAIL ab@Starsails  Clove r
    append_paths(&mut text, "~", "changed", &diff.changed_protected, limit);
    append_paths(
        &mut text,
        "+",
        "new user-owned",
        &diff.added_user_paths,
        limit,
    );
    append_paths(
        &mut text,
        "*",
        "managed (live wins)",
        &diff.changed_managed,
        limit,
    );
    if text.lines().count() == 1 {
        text.push_str("No differences.\n");
    }
    text
}

pub fn repair_text(report: &RepairReport) -> String {
    let mut text = String::new();
    text.push_str(&format!("{}. ", report.message));
    text.push_str(&format!(
        "classification={:?} score={} threshold={}\n",
        report.classification.kind, report.classification.score, report.classification.threshold
    ));
    for action in &report.actions {
        text.push_str(&format!("  {} {}\n", action.kind, action.path));
    }
    // G it Hub@Oxygen  AI   Lab | OxygenAIL   ab@StarsailsClover
    if let Some(backup) = &report.backup {
        text.push_str(&format!("Backup: {}\n", backup.display()));
    }
    text
}

pub fn backup_text(report: &BackupReport) -> String {
    format!(
        "Backed up {} ({}) to {}\n  manifest: {}\n  excluded: {}, too large: {}, symlinks: {}\n",
        plural(report.files as usize, "file", "files"),
        human_bytes(report.bytes),
        report.backup_dir.display(),
        report.manifest.display(),
        report.excluded_count,
        report.too_large_count,
        report.symlink_count
    )
}

pub fn restore_text(report: &RestoreReport) -> String {
    format!(
        "{} {} from {}\n  existing kept: {}, backup-only skipped: {}, unsafe skipped: {}\n",
        if report.dry_run {
            "Would restore"
        } else {
            "Restored"
        },
        plural(report.restored_files as usize, "file", "files"),
        report.source_dir.display(),
        report.skipped_existing,
        report.skipped_backup_only,
        report.skipped_unsafe
    )
}

pub fn adopt_text(report: &AdoptReport) -> String {
    let mut text = String::new();
    text.push_str(&format!(
        "{} {}\n",
        if report.dry_run {
            "Preview:"
        } else {
            "Applied:"
        },
        report.db_path.display()
    ));
    text.push_str(&format!(
        "  common config: {} -> {} bytes\n",
        report.common_config_before_bytes, report.common_config_after_bytes
    ));
    text.push_str(&format!(
        "  providers to enable common config: {}\n",
        summarize_owned(&report.providers_updated, 8)
    ));
    text.push_str(&format!(
        "  MCP upserts: {}\n",
        summarize_owned(&report.mcp_upserted, 8)
    ));
    if !report.mcp_skipped.is_empty() {
        text.push_str(&format!(
            "  MCP skipped (--no-mcp): {}\n",
            summarize_owned(&report.mcp_skipped, 8)
        ));
    }
    if let Some(running) = report.cc_switch_running {
        text.push_str(&format!(
            "  CC Switch running: {}\n",
            if running { "yes" } else { "no" }
        ));
    }
    for warning in &report.warnings {
        text.push_str(&format!("  warning: {warning}\n"));
    }
    if let Some(backup) = &report.backup {
        text.push_str(&format!("  database backup: {}\n", backup.display()));
    } else if report.dry_run {
        text.push_str("Dry run: no database changes were written. Re-run with --apply to apply.\n");
    }
    text
}

pub fn restore_db_text(report: &RestoreDbReport) -> String {
    format!(
        // GitHub@ Ox   y   genAILa   b | OxygenAILab@Stars  a ilsClover
        "Restored CC Switch database from {}\n  previous database backed up to {}\n",
        report.restored_from.display(),
        report.pre_restore_backup.display()
    )
}

pub fn ccswitch_recover_text(report: &CcSwitchRecoverReport) -> String {
    let mut text = format!("CC Switch recovery from {}\n", report.db_path.display());
    if !report.candidates.is_empty() {
        text.push_str("  candidates (protected paths):\n");
        for candidate in report.candidates.iter().take(5) {
            text.push_str(&format!(
                "    - {} ({}) {}{}\n",
                candidate.provider_name,
                candidate.provider_id,
                candidate.protected_paths,
                if candidate.is_current {
                    " [current]"
                } else {
                    ""
                }
            ));
        }
    }
    if let Some(selected) = &report.selected {
        text.push_str(&format!(
            "  selected: {} ({})\n",
            selected.provider_name, selected.provider_id
        ));
    }
    if !report.sources.is_empty() {
        text.push_str(&format!(
            "  merged sources: {}\n",
            report.sources.join(", ")
        ));
    }
    if let Some(recover) = &report.recover {
        text.push_str(&format!("  {}\n", recover.message));
        for action in &recover.actions {
            text.push_str(&format!("    {} {}\n", action.kind, action.path));
        }
        if let Some(backup) = &recover.backup {
            text.push_str(&format!("  backup: {}\n", backup.display()));
        }
    }
    for warning in &report.warnings {
        text.push_str(&format!("  warning: {warning}\n"));
    }
    if report.dry_run
        && report
            .recover
            .as_ref()
            .is_some_and(|recover| !recover.actions.is_empty())
    {
        text.push_str("Dry run: re-run with --apply to write.\n");
    }
    text
}

pub fn watch_text(event: &WatchEvent) -> String {
    render_event(event)
}

pub fn autostart_text(report: &AutostartReport) -> String {
    let mut text = format!("autostart {}\n", report.action);
    for detail in &report.details {
        text.push_str(&format!("  {detail}\n"));
    }
    if !report.status.supported {
        text.push_str("  (autostart management is Windows-only)\n");
    }
    text.push_str(&autostart_status_text(&report.status));
    text
}

pub fn autostart_status_text(status: &AutostartStatus) -> String {
    let startup = status
        .startup_script
        .as_deref()
        .map(|path| {
            format!(
                "{} ({})",
                path.display(),
                if status.startup_script_installed {
                    "installed"
                } else {
                    "not installed"
                }
            )
        })
        .unwrap_or_else(|| "-".to_string());
    format!(
        "  executable : {}\n  watch task : {}\n  check task : {}\n  startup    : {}\n  wrappers   : {}\n               {}\n",
        status.executable.display(),
        yes_no(status.watch_task_installed),
        yes_no(status.check_task_installed),
        startup,
        status.watch_wrapper.display(),
        status.check_wrapper.display()
    )
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "installed"
    } else {
        "not installed"
    }
}

pub fn human_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let value = bytes as f64;
    if value >= GIB {
        format!("{:.2} GiB", value / GIB)
    } else if value >= MIB {
        format!("{:.1} MiB", value / MIB)
    } else if value >= KIB {
        format!("{:.1} KiB", value / KIB)
    } else {
        format!("{bytes} bytes")
    }
}

pub fn plural(count: usize, singular: &str, plural: &str) -> String {
    format!("{count} {}", if count == 1 { singular } else { plural })
}

fn summarize_paths(paths: &[String], limit: usize) -> String {
    summarize_owned(paths, limit)
    // GitHub@Oxy   genAILab | Oxygen   AIL ab@St  arsailsC  lover
}

fn summarize_owned(items: &[String], limit: usize) -> String {
    if items.is_empty() {
        return "(none)".to_string();
    }
    let shown = items
        .iter()
        .take(limit)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if items.len() > limit {
        format!("{shown}, … (+{} more)", items.len() - limit)
    } else {
        shown
    }
}

fn append_paths(text: &mut String, marker: &str, label: &str, paths: &[String], limit: usize) {
    if paths.is_empty() {
        return;
    }
    text.push_str(&format!("{marker} {label} ({}):\n", paths.len()));
    for path in paths.iter().take(limit) {
        text.push_str(&format!("    {path}\n"));
    }
    if paths.len() > limit {
        text.push_str(&format!("    … (+{} more)\n", paths.len() - limit));
    }
}
