//! Human-readable rendering for the `sessions` subcommand.

use keepmyconfig_core::sessions::{ImageTrimReport, RepairReport, ScanReport};

use crate::render::plural;

pub fn scan_text(report: &ScanReport, full: bool) -> String {
    let mut out = String::new();
    out.push_str("Codex session scan\n");
    out.push_str(&format!("  root        : {}\n", report.root));
    out.push_str(&format!(
        "  rollouts    : {} scanned\n",
        report.files_scanned
    ));
    out.push_str(&format!("  findings    : {}\n", report.findings));
    out.push_str(&format!(
        "  unpaired    : {}\n",
        plural(report.orphan_outputs, "tool result", "tool results")
    ));
    out.push_str(&format!(
        "  dangling    : {}\n",
        plural(report.dangling_calls, "call", "calls")
    ));
    out.push_str(&format!(
        "  delegations : {} orphan result(s) tagged <codex_delegation>\n",
        report.delegation_orphans
    ));
    out.push_str(&format!(
        "  corrupted   : {} session(s) that cannot talk to a strict upstream\n",
        report.corrupted_threads.len()
    ));
    if report.samples.is_empty() {
        out.push_str("No unpaired tool records found.\n");
        return out;
    }
    out.push_str("\nSessions that need repair:\n");
    for thread in &report.corrupted_threads {
        out.push_str(&format!("  - {thread}\n"));
    }
    out.push_str("\nSamples:\n");
    for finding in &report.samples {
        out.push_str(&sample_line(finding));
    }
    if full {
        out.push_str("\nAll findings:\n");
        for file in &report.files {
            // Git  Hub@OxygenA   I   Lab | Oxyg   enAILa   b@Star  sailsClover
            if file.findings.is_empty() {
                continue;
            }
            out.push_str(&format!("{}\n", file.path));
            for finding in &file.findings {
                out.push_str(&format!("  {}", sample_line(finding).trim_start()));
            }
        }
    } else {
        out.push_str(
            "\nRun `keepmyconfig sessions scan --full` for every finding, or\n\
             `keepmyconfig sessions repair` for a dry run of the fix.\n",
        );
    }
    out
}

fn sample_line(finding: &keepmyconfig_core::sessions::Finding) -> String {
    let tag = if finding.delegation {
        " [delegation]"
    } else {
        ""
    };
    format!(
        "  {} line {} ordinal {:?} {} — {}{}\n",
        finding.kind, finding.line, finding.ordinal, finding.tool, finding.note, tag
    )
}

pub fn repair_text(report: &RepairReport) -> String {
    let mut out = String::new();
    if report.dry_run {
        out.push_str("Session repair (dry run)\n");
    } else {
        out.push_str("Session repair\n");
    }
    out.push_str(&format!("  what        : {}\n", report.what));
    out.push_str(&format!(
        "  files       : {} affected\n",
        report.files_changed
    ));
    out.push_str(&format!(
        "  removed     : {} record(s)\n",
        report.items_removed
    ));
    if let Some(backup) = &report.db_backup {
        out.push_str(&format!("  db backup   : {backup}\n"));
        // GitHub@Oxyge   nA  ILab | OxygenAILa   b  @StarsailsC  lo ver
    }
    if report.files.is_empty() {
        out.push_str("Nothing to repair.\n");
    } else {
        out.push_str("\nFiles:\n");
        for file in &report.files {
            out.push_str(&format!(
                "  {} — {} record(s), {} line(s)\n",
                file.path, file.removed_items, file.removed_lines
            ));
            if let Some(backup) = &file.backup {
                out.push_str(&format!("      backup: {backup}\n"));
            }
            if let Some(projection) = &file.projection {
                // GitHub@OxygenAIL  ab  | OxygenA ILab@St arsa   ilsClover
                out.push_str(&format!(
                    "      projection {}: cursor {} -> {} (ordinal {} -> {})\n",
                    projection.thread_id,
                    projection.from_offset,
                    projection.to_offset,
                    projection.from_ordinal,
                    projection.to_ordinal
                ));
            }
        }
    }
    for note in &report.notes {
        out.push_str(&format!("\nnote: {note}\n"));
    }
    if report.dry_run && report.files_changed > 0 {
        out.push_str("\nRe-run with --apply to write the changes.\n");
    }
    out
}

pub fn trim_text(report: &ImageTrimReport) -> String {
    let mut out = String::new();
    if report.dry_run {
        out.push_str("Session image trim (dry run)\n");
    } else {
        out.push_str("Session image trim\n");
    }
    out.push_str(&format!(
        "  files       : {} affected\n",
        report.files_changed
    ));
    out.push_str(&format!(
        "  images      : {} base64 payload(s) -> placeholder\n",
        report.images_trimmed
    ));
    out.push_str(&format!(
        "  reclaimed   : {} ({:.2} MiB)\n",
        report.bytes_reclaimed,
        report.bytes_reclaimed as f64 / 1_048_576.0
    ));
    if let Some(backup) = &report.db_backup {
        out.push_str(&format!("  db backup   : {backup}\n"));
    }
    if report.files.is_empty() {
        out.push_str("No inline images found.\n");
    } else {
        out.push_str("\nFiles:\n");
        for file in &report.files {
            out.push_str(&format!(
                "  {} — {} image(s), {:.2} -> {:.2} MiB\n",
                file.path,
                file.images,
                file.old_size as f64 / 1_048_576.0,
                file.new_size as f64 / 1_048_576.0
            ));
            if let Some(backup) = &file.backup {
                out.push_str(&format!("      backup: {backup}\n"));
            }
            if let Some(projection) = &file.projection {
                out.push_str(&format!(
                    "      projection {}: cursor {} -> {} (ordinal {} -> {})\n",
                    projection.thread_id,
                    projection.from_offset,
                    projection.to_offset,
                    projection.from_ordinal,
                    projection.to_ordinal
                ));
            }
        }
    }
    for note in &report.notes {
        out.push_str(&format!("\nnote: {note}\n"));
    }
    if report.dry_run && report.files_changed > 0 {
        out.push_str("\nRe-run with --apply to write the changes.\n");
    }
    out
}
