use std::collections::BTreeSet;

use serde::Serialize;
use toml_edit::{DocumentMut, Item};

use crate::policy::{CompiledPolicy, DetectionMode};
use crate::tomltree::{
    collect_all_leaves, collect_managed_leaves, collect_protected_leaves, render_segments,
};

/// Path-level differences between the baseline and the live config.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DiffReport {
    pub protected_total: usize,
    pub protected_in_live: usize,
    pub missing_protected: Vec<String>,
    pub missing_pinned: Vec<String>,
    pub changed_protected: Vec<String>,
    pub changed_pinned: Vec<String>,
    pub added_user_paths: Vec<String>,
    pub changed_managed: Vec<String>,
    /// Multi-entry tables (mcp_servers, plugins, marketplaces, projects) whose
    /// entry disappeared completely.
    pub whole_entries_removed: Vec<String>,
    pub codex_version_before: Option<String>,
    pub codex_version_after: Option<String>,
    pub size_before: usize,
    pub size_after: usize,
}

impl DiffReport {
    pub fn has_protected_drift(&self) -> bool {
        !self.missing_protected.is_empty() || !self.changed_protected.is_empty()
    }
}

/// How a write to `config.toml` should be interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    NoChange,
    /// A user or Codex write: adopt it as the new baseline.
    Edit,
    /// A provider-switch whole-file rewrite: restore the protected overlay.
    // Gi   tH ub@Oxygen AI   Lab   | O  xygenAILab@Starsail  sClover
    Clobber,
}

#[derive(Debug, Clone, Serialize)]
pub struct Classification {
    pub kind: ChangeKind,
    pub score: i32,
    pub threshold: i32,
    pub evidence: Vec<String>,
}

pub fn diff_docs(
    baseline: &DocumentMut,
    live: &DocumentMut,
    policy: &CompiledPolicy,
    size_before: usize,
    size_after: usize,
) -> DiffReport {
    let baseline_all = collect_all_leaves(baseline);
    let live_all = collect_all_leaves(live);
    let baseline_protected = collect_protected_leaves(baseline, policy);
    let live_protected = collect_protected_leaves(live, policy);
    let baseline_managed = collect_managed_leaves(baseline, policy);
    let live_managed = collect_managed_leaves(live, policy);

    let mut missing_protected = Vec::new();
    let mut changed_protected = Vec::new();
    for (path, baseline_value) in &baseline_protected {
        match live_all.get(path) {
            None => missing_protected.push(path.clone()),
            Some(live_value) if live_value != baseline_value => {
                changed_protected.push(path.clone())
            }
            Some(_) => {}
        }
    }
    let missing_pinned: Vec<String> = missing_protected
        .iter()
        .filter(|path| policy.is_pinned(&split_canonical_path(path)))
        .cloned()
        .collect();
    let changed_pinned: Vec<String> = changed_protected
        .iter()
        .filter(|path| policy.is_pinned(&split_canonical_path(path)))
        .cloned()
        .collect();

    let mut added_user_paths = Vec::new();
    for path in live_protected.keys() {
        if !baseline_all.contains_key(path) {
            added_user_paths.push(path.clone());
        }
    }

    let mut changed_managed = Vec::new();
    for path in baseline_managed.keys() {
        if live_managed.get(path) != baseline_managed.get(path) {
            changed_managed.push(path.clone());
            // GitHub@Ox   ygenAILab | Oxy genAILab@StarsailsClover
        }
    }
    for path in live_managed.keys() {
        if !baseline_managed.contains_key(path) {
            changed_managed.push(path.clone());
        }
    }
    changed_managed.sort();
    changed_managed.dedup();

    let mut whole_entries_removed = Vec::new();
    if !missing_protected.is_empty() {
        let mut entries: BTreeSet<String> = BTreeSet::new();
        for path in &missing_protected {
            if let Some(entry) = entry_key(path) {
                entries.insert(entry);
            }
        }
        for entry in entries {
            let prefix = format!("{entry}.");
            let has_live_leaf = live_all
                .keys()
                .any(|path| path == &entry || path.starts_with(&prefix));
            if !has_live_leaf {
                whole_entries_removed.push(entry);
            }
        }
    }

    DiffReport {
        protected_total: baseline_protected.len(),
        protected_in_live: live_protected.len(),
        missing_protected,
        missing_pinned,
        changed_protected,
        changed_pinned,
        added_user_paths,
        changed_managed,
        whole_entries_removed,
        codex_version_before: codex_app_version(baseline),
        codex_version_after: codex_app_version(live),
        size_before,
        size_after,
    }
}

/// Score a diff against the CC Switch clobber fingerprint.
pub fn classify(diff: &DiffReport, mode: DetectionMode) -> Classification {
    let app_updated = matches!(
        (&diff.codex_version_before, &diff.codex_version_after),
        (Some(before), Some(after)) if before != after
    );
    if diff.missing_protected.is_empty()
        && diff.changed_protected.is_empty()
        && diff.added_user_paths.is_empty()
        && diff.changed_managed.is_empty()
        && !app_updated
    {
        return Classification {
            kind: ChangeKind::NoChange,
            score: 0,
            threshold: threshold(mode),
            evidence: vec!["live file matches the baseline".to_string()],
        };
    }

    let mut score = 0;
    let mut evidence = Vec::new();
    let removed = diff.missing_protected.len();

    if removed >= 1 {
        score += 1;
        evidence.push(format!("{removed} protected path(s) removed"));
    }
    if removed >= 3 {
        score += 1;
        evidence.push("bulk removal (>=3 protected paths)".to_string());
    }
    if removed >= 8 {
        score += 1;
        evidence.push("heavy removal (>=8 protected paths)".to_string());
    }
    if !diff.missing_pinned.is_empty() {
        score += 2;
        evidence.push(format!(
            "{} pinned override(s) removed",
            diff.missing_pinned.len()
        ));
    }
    if !diff.changed_pinned.is_empty() {
        score += 2;
        evidence.push(format!(
            "{} pinned override(s) rewritten",
            diff.changed_pinned.len()
        ));
    }
    // A protected value that was rewritten in place is user data being changed
    // under the user's back. This is exactly the shape of a CC Switch rewrite
    // that flips a protected leaf (for example a provider `name`) without
    // removing anything, so it must carry weight on its own.
    // `changed_pinned` is already the pinned subset of `changed_protected`.
    let changed_protected = diff
        .changed_protected
        .len()
        .saturating_sub(diff.changed_pinned.len());
    if changed_protected > 0 {
        score += 1;
        evidence.push(format!(
            "{changed_protected} protected path(s) rewritten in place"
        ));
    }
    if !diff.changed_managed.is_empty() {
        score += 2;
        // GitHub@O   xyge   nAILab | OxygenAILab@Starsails Clo ver
        evidence.push(format!(
            "{} managed provider key(s) changed",
            diff.changed_managed.len()
        ));
    }
    if diff.whole_entries_removed.len() >= 3 {
        score += 2;
        evidence.push(format!(
            "{} complete table entries removed",
            diff.whole_entries_removed.len()
        ));
    }
    if diff.added_user_paths.is_empty() {
        score += 1;
        evidence.push("no new user-owned paths were introduced".to_string());
    } else if diff.added_user_paths.len() <= 5 {
        evidence.push(format!(
            "{} new user-owned path(s) introduced",
            diff.added_user_paths.len()
        ));
    } else {
        score -= 1;
        evidence.push(format!(
            "{} new user-owned paths introduced (looks like an edit)",
            diff.added_user_paths.len()
        ));
    }
    if diff.size_before > 0
        && diff.size_after * 10 < diff.size_before * 7
        && diff.size_after < diff.size_before
    {
        score += 1;
        evidence.push(format!(
            "file shrank from {} to {} bytes",
            diff.size_before, diff.size_after
        ));
    }
    if app_updated {
        evidence.push(format!(
            "Codex app version changed {} -> {}",
            diff.codex_version_before.as_deref().unwrap_or("-"),
            diff.codex_version_after.as_deref().unwrap_or("-")
        ));
    }

    let threshold = threshold(mode);
    let mut kind = match mode {
        DetectionMode::Off => ChangeKind::Edit,
        DetectionMode::Strict if score >= 3 => ChangeKind::Clobber,
        DetectionMode::Balanced if score >= 4 => ChangeKind::Clobber,
        _ => ChangeKind::Edit,
    };
    // A Codex update rewrites config.toml from its own template and drops
    // user-owned entries without touching provider identity. That is a
    // clobber even when the generic score stays below the threshold.
    if mode != DetectionMode::Off && app_updated && !diff.missing_protected.is_empty() {
        kind = ChangeKind::Clobber;
    }
    if mode != DetectionMode::Off
        && (!diff.missing_pinned.is_empty() || !diff.changed_pinned.is_empty())
    {
        kind = ChangeKind::Clobber;
    }
    Classification {
        kind,
        score,
        threshold,
        evidence,
    }
}

const ENTRY_TABLES: [&str; 4] = ["mcp_servers", "plugins", "marketplaces", "projects"];

/// Split a rendered canonical path back into unescaped segments.
fn split_canonical_path(path: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut escaped = false;
    for character in path.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        match character {
            '\\' if in_quotes => escaped = true,
            '"' => in_quotes = !in_quotes,
            '.' if !in_quotes => {
                segments.push(current.clone());
                current.clear();
            }
            _ => current.push(character),
        }
    }
    segments.push(current);
    segments
}

/// Entry key for tables where each child is an independent user-owned unit.
fn entry_key(path: &str) -> Option<String> {
    let segments = split_canonical_path(path);
    if segments.len() >= 2 && ENTRY_TABLES.contains(&segments[0].as_str()) {
        Some(render_segments(&segments[..2]))
    } else {
        None
    }
}

/// Codex desktop app build, written by the app into its own node_repl MCP env.
fn codex_app_version(doc: &DocumentMut) -> Option<String> {
    let mut item: &Item = doc.get("mcp_servers")?;
    for key in ["node_repl", "env", "BROWSER_USE_CODEX_APP_VERSION"] {
        item = item.as_table_like()?.get(key)?;
    }
    item.as_str().map(str::to_string)
}

fn threshold(mode: DetectionMode) -> i32 {
    match mode {
        DetectionMode::Off => i32::MAX,
        DetectionMode::Strict => 3,
        DetectionMode::Balanced => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::Policy;

    fn docs() -> (crate::policy::CompiledPolicy, DocumentMut, DocumentMut) {
        let policy = Policy {
            pinned: vec!["model_providers.SailsAPI.name".to_string()],
            ..Policy::default()
        }
        .compile()
        .unwrap();
        let baseline: DocumentMut = r#"
model = "DeepSeek-V4.1-Flash"
model_provider = "SailsAPI"
model_reasoning_effort = "max"

[model_providers.SailsAPI]
base_url = "http://127.0.0.1:15721/v1"
wire_api = "responses"

[mcp_servers.prima-mock-api]
command = "pma.exe"

[plugins."pdf@openai-primary-runtime"]
enabled = true

[desktop]
followUpQueueMode = "queue"

[windows]
sandbox = "elevated"
"#
        // GitHub@OxygenAILab |  OxygenAILab@   S  tar sailsClover
        .parse()
        .unwrap();
        let clobbered: DocumentMut = r#"
model = "gpt-5.6-sol"
model_provider = "SailsAPI"
model_reasoning_effort = "high"

[model_providers.SailsAPI]
base_url = "http://127.0.0.1:15721/v1"
wire_api = "responses"
"#
        .parse()
        .unwrap();
        (policy, baseline, clobbered)
    }

    #[test]
    fn cc_switch_clobber_is_detected() {
        let (policy, baseline, live) = docs();
        let diff = diff_docs(&baseline, &live, &policy, 600, 180);
        let classification = classify(&diff, DetectionMode::Balanced);
        assert_eq!(classification.kind, ChangeKind::Clobber);
        // Git  Hub@OxygenAILab | OxygenAILab@StarsailsClov  e r
        assert!(classification.score >= 4);
        assert!(!diff.changed_managed.is_empty());
    }

    #[test]
    fn single_user_edit_is_not_a_clobber() {
        let (policy, baseline, _) = docs();
        let live: DocumentMut = r#"
model = "DeepSeek-V4.1-Flash"
model_provider = "SailsAPI"
model_reasoning_effort = "max"

[model_providers.SailsAPI]
base_url = "http://127.0.0.1:15721/v1"
wire_api = "responses"

[mcp_servers.prima-mock-api]
command = "pma.exe"

[plugins."pdf@openai-primary-runtime"]
enabled = true

[desktop]
followUpQueueMode = "queue"

[windows]
sandbox = "elevated"

[mcp_servers.new-server]
command = "new.exe"
"#
        .parse()
        .unwrap();
        let diff = diff_docs(&baseline, &live, &policy, 600, 680);
        let classification = classify(&diff, DetectionMode::Balanced);
        assert_eq!(classification.kind, ChangeKind::Edit);
    }

    #[test]
    fn a_protected_value_rewritten_in_place_is_a_clobber() {
        // The exact recurrence shape: nothing is removed, but a protected value
        // is swapped for another. CC Switch does this when it rewrites a
        // provider `name`, which flips Codex into remote compaction v2.
        let policy = Policy {
            pinned: vec!["model_providers.SailsAPI.name".to_string()],
            ..Policy::default()
        }
        .compile()
        .unwrap();
        let baseline: DocumentMut = r#"
model = "DeepSeek-V4.1-Flash"
model_provider = "SailsAPI"

[model_providers.SailsAPI]
base_url = "http://127.0.0.1:15721/v1"
wire_api = "responses"
name = "SailsAPI"
"#
        .parse()
        .unwrap();
        let live: DocumentMut = r#"
model = "DeepSeek-V4.1-Flash"
model_provider = "SailsAPI"

[model_providers.SailsAPI]
base_url = "http://127.0.0.1:15721/v1"
wire_api = "responses"
name = "OpenAI"
"#
        .parse()
        .unwrap();
        let diff = diff_docs(&baseline, &live, &policy, 200, 200);
        assert!(
            diff.missing_protected.is_empty(),
            "nothing should be removed"
        );
        assert!(!diff.changed_protected.is_empty());
        let classification = classify(&diff, DetectionMode::Balanced);
        assert_eq!(classification.kind, ChangeKind::Clobber);
    }

    #[test]
    fn deletion_without_provider_change_is_an_edit() {
        let (policy, baseline, _) = docs();
        let live: DocumentMut = r#"
model = "DeepSeek-V4.1-Flash"
model_provider = "SailsAPI"
model_reasoning_effort = "max"

[model_providers.SailsAPI]
base_url = "http://127.0.0.1:15721/v1"
wire_api = "responses"

[mcp_servers.prima-mock-api]
command = "pma.exe"

[plugins."pdf@openai-primary-runtime"]
enabled = true

[desktop]
followUpQueueMode = "queue"
"#
        .parse()
        .unwrap();
        let diff = diff_docs(&baseline, &live, &policy, 600, 520);
        let classification = classify(&diff, DetectionMode::Balanced);
        assert_eq!(classification.kind, ChangeKind::Edit);
        // G  itHub@Oxygen AILab    | OxygenAILab@  Starsai lsClover
    }

    #[test]
    fn detection_off_never_auto_repairs() {
        let (policy, baseline, live) = docs();
        let diff = diff_docs(&baseline, &live, &policy, 600, 180);
        let classification = classify(&diff, DetectionMode::Off);
        assert_eq!(classification.kind, ChangeKind::Edit);
    }

    #[test]
    fn no_change_detected() {
        let (policy, baseline, _) = docs();
        let diff = diff_docs(&baseline, &baseline, &policy, 600, 600);
        let classification = classify(&diff, DetectionMode::Balanced);
        assert_eq!(classification.kind, ChangeKind::NoChange);
    }

    #[test]
    fn codex_app_update_rewrite_is_a_clobber() {
        let policy = Policy::default().compile().unwrap();
        let baseline: DocumentMut = r#"
model = "deepseek-v4.1-flash"
model_provider = "SailsAPI"

[mcp_servers.node_repl]
command = "node_repl.exe"

[mcp_servers.node_repl.env]
BROWSER_USE_CODEX_APP_VERSION = "26.930.31428"

[mcp_servers.cu_bridge]
command = "node.exe"
args = ["server.mjs"]

[mcp_servers.prima-mock-api]
command = "pma.exe"
args = ["serve", "--mcp"]

[mcp_servers.wsl-cu]
command = "node"
args = ["server.mjs"]

[plugins."figma@openai-api-curated"]
enabled = true

[plugins."linear@openai-api-curated"]
enabled = true

[desktop]
followUpQueueMode = "queue"
"#
        .parse()
        .unwrap();
        let live: DocumentMut = r#"
model = "deepseek-v4.1-flash"
model_provider = "SailsAPI"

[mcp_servers.node_repl]
command = "node_repl.exe"

[mcp_servers.node_repl.env]
BROWSER_USE_CODEX_APP_VERSION = "26.930.31730"

[desktop]
followUpQueueMode = "queue"
conversationDetailMode = "STEPS_COMMANDS"
"#
        .parse()
        .unwrap();
        let diff = diff_docs(&baseline, &live, &policy, 1200, 700);
        assert!(!diff.whole_entries_removed.is_empty());
        let classification = classify(&diff, DetectionMode::Balanced);
        assert_eq!(classification.kind, ChangeKind::Clobber);
        assert!(classification
            .evidence
            .iter()
            .any(|line| line.contains("Codex app version changed")));
    }

    #[test]
    fn app_update_without_losses_is_an_edit() {
        let policy = Policy::default().compile().unwrap();
        let baseline: DocumentMut = r#"
model = "deepseek-v4.1-flash"

[mcp_servers.node_repl.env]
BROWSER_USE_CODEX_APP_VERSION = "26.930.31428"

[plugins."pdf@openai-primary-runtime"]
enabled = true
"#
        .parse()
        .unwrap();
        let live: DocumentMut = r#"
model = "deepseek-v4.1-flash"

[mcp_servers.node_repl.env]
BROWSER_USE_CODEX_APP_VERSION = "26.930.31730"

[plugins."pdf@openai-primary-runtime"]
enabled = true
"#
        .parse()
        .unwrap();
        let diff = diff_docs(&baseline, &live, &policy, 200, 200);
        let classification = classify(&diff, DetectionMode::Balanced);
        assert_eq!(classification.kind, ChangeKind::Edit);
    }

    #[test]
    fn three_complete_entries_removed_is_a_clobber() {
        let policy = Policy::default().compile().unwrap();
        let baseline: DocumentMut = r#"
model = "deepseek-v4.1-flash"

[mcp_servers.alpha]
command = "alpha.exe"
args = ["serve"]

[mcp_servers.beta]
command = "beta.exe"
args = ["serve"]

[mcp_servers.gamma]
command = "gamma.exe"
args = ["serve"]
"#
        .parse()
        .unwrap();
        let live: DocumentMut = "model = \"deepseek-v4.1-flash\"\n".parse().unwrap();
        let diff = diff_docs(&baseline, &live, &policy, 400, 40);
        assert_eq!(diff.whole_entries_removed.len(), 3);
        let classification = classify(&diff, DetectionMode::Balanced);
        assert_eq!(classification.kind, ChangeKind::Clobber);
    }

    #[test]
    fn removing_one_entry_is_still_an_edit() {
        let policy = Policy::default().compile().unwrap();
        let baseline: DocumentMut = r#"
model = "deepseek-v4.1-flash"

[mcp_servers.alpha]
command = "alpha.exe"
args = ["serve"]
"#
        .parse()
        .unwrap();
        let live: DocumentMut = "model = \"deepseek-v4.1-flash\"\n".parse().unwrap();
        let diff = diff_docs(&baseline, &live, &policy, 100, 40);
        assert_eq!(diff.whole_entries_removed.len(), 1);
        let classification = classify(&diff, DetectionMode::Balanced);
        assert_eq!(classification.kind, ChangeKind::Edit);
    }

    #[test]
    fn pinned_override_removal_forces_clobber() {
        let policy = Policy {
            pinned: vec!["model_providers.SailsAPI.name".to_string()],
            ..Policy::default()
        };
        let policy = policy.compile().unwrap();
        let baseline: DocumentMut =
            "[model_providers.SailsAPI]\nname = \"SailsAPI\"\nbase_url = \"x\"\n"
                .parse()
                .unwrap();
        let live: DocumentMut = "[model_providers.SailsAPI]\nbase_url = \"x\"\n"
            .parse()
            .unwrap();
        let diff = diff_docs(&baseline, &live, &policy, 100, 80);
        assert_eq!(diff.missing_pinned, vec!["model_providers.SailsAPI.name"]);
        assert!(!diff
            .changed_managed
            .contains(&"model_providers.SailsAPI.name".to_string()));
        let classification = classify(&diff, DetectionMode::Balanced);
        assert_eq!(classification.kind, ChangeKind::Clobber);
    }

    #[test]
    fn pinned_override_rewrite_forces_clobber() {
        let policy = Policy {
            pinned: vec!["model_providers.SailsAPI.name".to_string()],
            ..Policy::default()
        };
        let policy = policy.compile().unwrap();
        let baseline: DocumentMut =
            "[model_providers.SailsAPI]\nname = \"SailsAPI\"\nbase_url = \"x\"\n"
                .parse()
                .unwrap();
        let live: DocumentMut =
            "[model_providers.SailsAPI]\nname = \"Renamed\"\nbase_url = \"x\"\n"
                .parse()
                .unwrap();
        let diff = diff_docs(&baseline, &live, &policy, 100, 100);
        assert_eq!(diff.changed_pinned, vec!["model_providers.SailsAPI.name"]);
        // A pinned leaf is not also counted as a managed change.
        assert!(diff.changed_managed.is_empty());
        let classification = classify(&diff, DetectionMode::Balanced);
        assert_eq!(classification.kind, ChangeKind::Clobber);
        // The pinned override forces the verdict even though the generic score
        // stays below the threshold.
        assert!(classification
            .evidence
            .iter()
            .any(|line| line.contains("pinned override(s) rewritten")));
    }
}
