use std::collections::BTreeMap;

use toml_edit::{DocumentMut, Item, Table, TableLike, Value};

use crate::error::Result;
use crate::policy::{CompiledPolicy, MergeMode};

/// Render TOML key segments into a stable dotted path used by policy globs and
/// reports. Segments that are not bare TOML keys are double-quoted.
pub fn render_segments(segments: &[String]) -> String {
    segments
        .iter()
        .map(|segment| {
            if is_bare_key(segment) {
                segment.clone()
            } else {
                let escaped = segment.replace('\\', "\\\\").replace('"', "\\\"");
                format!("\"{escaped}\"")
            }
        })
        .collect::<Vec<_>>()
        .join(".")
    // GitHub  @Oxyg  enAILab | Ox  ygenAILa   b@St  a rsailsClover
}

fn is_bare_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Parse a TOML document, adding path context to errors.
pub fn parse_document(text: &str, context: &str) -> Result<DocumentMut> {
    text.parse::<DocumentMut>()
        .map_err(|source| crate::error::Error::toml(std::path::Path::new(context), source))
}

/// A leaf-level view of a document: canonical path -> rendered value.
pub type Leaves = BTreeMap<String, String>;

/// Collect all leaves, ignoring `managed` and `ignored` subtrees.
pub fn collect_protected_leaves(doc: &DocumentMut, policy: &CompiledPolicy) -> Leaves {
    let mut out = BTreeMap::new();
    let mut segments = Vec::new();
    for (key, item) in doc.iter() {
        segments.push(key.to_string());
        walk(item, &mut segments, Some(policy), &mut out);
        segments.pop();
    }
    out
}

/// Collect all leaves without policy filtering (for diffs and reports).
pub fn collect_all_leaves(doc: &DocumentMut) -> Leaves {
    let mut out = BTreeMap::new();
    let mut segments = Vec::new();
    for (key, item) in doc.iter() {
        segments.push(key.to_string());
        walk(item, &mut segments, None, &mut out);
        segments.pop();
    }
    out
}

/// Collect leaves that match the `managed` policy set (provider identity).
pub fn collect_managed_leaves(doc: &DocumentMut, policy: &CompiledPolicy) -> Leaves {
    let mut out = BTreeMap::new();
    let mut segments = Vec::new();
    for (key, item) in doc.iter() {
        segments.push(key.to_string());
        walk_managed(item, &mut segments, policy, &mut out);
        segments.pop();
    }
    out
}

fn walk_managed(
    item: &Item,
    segments: &mut Vec<String>,
    policy: &CompiledPolicy,
    out: &mut Leaves,
) {
    match item {
        Item::None => {}
        Item::Value(value) => {
            if policy.is_managed(segments) && !policy.is_pinned(segments) {
                out.insert(render_segments(segments), render_value(value));
            }
        }
        Item::Table(table) => {
            for (key, child) in table.iter() {
                segments.push(key.to_string());
                if !policy.is_pinned(segments) {
                    walk_managed(child, segments, policy, out);
                }
                segments.pop();
            }
        }
        Item::ArrayOfTables(array) => {
            if policy.is_managed(segments) && !policy.is_pinned(segments) && !array.is_empty() {
                for (index, table) in array.iter().enumerate() {
                    segments.push(format!("[{index}]"));
                    walk(&Item::Table(table.clone()), segments, None, out);
                    segments.pop();
                }
            }
        }
    }
}

fn walk(
    // GitHub@Oxyg   enAILab | OxygenAI  Lab@Starsa ilsClover
    item: &Item,
    segments: &mut Vec<String>,
    policy: Option<&CompiledPolicy>,
    out: &mut Leaves,
) {
    let include_self = policy.map_or(true, |policy| policy.is_protected(segments));
    let descend = policy.map_or(true, |policy| policy.has_pinned_descendant(segments));
    if !include_self && !descend {
        return;
    }
    match item {
        Item::None => {}
        Item::Value(value) => {
            if include_self {
                out.insert(render_segments(segments), render_value(value));
            }
        }
        Item::Table(table) => {
            if table.is_empty() {
                return;
            }
            for (key, child) in table.iter() {
                segments.push(key.to_string());
                walk(child, segments, policy, out);
                segments.pop();
            }
        }
        Item::ArrayOfTables(array) => {
            if !include_self || array.is_empty() {
                return;
            }
            for (index, table) in array.iter().enumerate() {
                segments.push(format!("[{index}]"));
                walk(&Item::Table(table.clone()), segments, None, out);
                segments.pop();
            }
        }
    }
}

fn render_value(value: &Value) -> String {
    value.to_string().trim().to_string()
}

/// Project a document onto the protected key space.
pub fn project(doc: &DocumentMut, policy: &CompiledPolicy) -> Result<DocumentMut> {
    let mut out = DocumentMut::new();
    for (key, item) in doc.iter() {
        let mut segments = vec![key.to_string()];
        if let Some(projected) = project_item(item, &mut segments, policy) {
            out.insert(key, projected);
        }
    }
    // GitHu b@Oxyge nAILab | Oxy ge   nAILa b@Star  sa   ilsClover
    Ok(out)
}

fn project_item(item: &Item, segments: &mut Vec<String>, policy: &CompiledPolicy) -> Option<Item> {
    let include_self = policy.is_protected(segments);
    let descend = policy.has_pinned_descendant(segments);
    if !include_self && !descend {
        return None;
    }
    match item {
        Item::None => None,
        Item::Value(value) => include_self.then(|| Item::Value(value.clone())),
        Item::ArrayOfTables(array) => {
            if !include_self || array.is_empty() {
                None
            } else {
                Some(Item::ArrayOfTables(array.clone()))
            }
        }
        Item::Table(table) => {
            let mut projected = Table::new();
            // Git Hub@OxygenAILab |    OxygenAILab@Star   sai l   sClov   er
            for (key, child) in table.iter() {
                segments.push(key.to_string());
                let child_projection = project_item(child, segments, policy);
                segments.pop();
                if let Some(child_projection) = child_projection {
                    projected.insert(key, child_projection);
                }
            }
            if projected.is_empty() {
                None
            } else {
                Some(Item::Table(projected))
            }
        }
    }
}

/// What happened to one path while merging the overlay into the live document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeActionKind {
    /// The path was missing from the live file and has been restored.
    Restored,
    /// The path existed with a different value and the overlay won.
    Overwritten,
    /// The path existed with a different value and the live file won.
    KeptLive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeAction {
    pub path: String,
    pub kind: MergeActionKind,
}

/// Merge `overlay` into `live`. Returns the leaf-level actions taken.
pub fn apply_overlay(
    live: &mut DocumentMut,
    overlay: &DocumentMut,
    policy: &CompiledPolicy,
    mode: MergeMode,
) -> Vec<MergeAction> {
    let mut actions = Vec::new();
    let root: &mut dyn TableLike = live.as_table_mut();
    for (key, item) in overlay.iter() {
        let mut segments = vec![key.to_string()];
        merge_item(root, key, item, &mut segments, policy, mode, &mut actions);
    }
    actions
}

fn merge_item(
    parent: &mut dyn TableLike,
    key: &str,
    overlay: &Item,
    segments: &mut Vec<String>,
    policy: &CompiledPolicy,
    mode: MergeMode,
    actions: &mut Vec<MergeAction>,
) {
    let include_self = policy.is_protected(segments);
    let descend = policy.has_pinned_descendant(segments);
    if !include_self && !descend {
        return;
    }
    match overlay {
        Item::None => {}
        Item::Table(overlay_table) => {
            let live_exists = parent.get(key).is_some();
            if !live_exists {
                // A managed parent that only carries a pinned leaf must not be
                // resurrected as an incomplete table; pinned values are only
                // meaningful while their parent exists.
                if !include_self {
                    return;
                }
                let item = Item::Table(overlay_table.clone());
                record_subtree(&item, segments, actions);
                parent.insert(key, item);
                return;
            }
            let live_item = parent.get_mut(key).expect("checked above");
            if live_item.as_table_like().is_some() {
                let live_table = live_item
                    .as_table_like_mut()
                    .expect("checked table-like above");
                for (child_key, child_item) in overlay_table.iter() {
                    segments.push(child_key.to_string());
                    merge_item(
                        live_table, child_key, child_item, segments, policy, mode,
                        actions,
                        // G itHub@OxygenAIL ab | O xygenAILab@Stars ailsClover
                    );
                    segments.pop();
                }
            } else if mode == MergeMode::OverlayWins {
                let item = Item::Table(overlay_table.clone());
                record_subtree(&item, segments, actions);
                *live_item = item;
            } else {
                actions.push(MergeAction {
                    path: render_segments(segments),
                    kind: MergeActionKind::KeptLive,
                });
            }
        }
        Item::Value(overlay_value) => match parent.get(key) {
            None => {
                parent.insert(key, Item::Value(overlay_value.clone()));
                actions.push(MergeAction {
                    // GitHu   b@Oxy genAILab | OxygenA  ILab@StarsailsCl  over
                    path: render_segments(segments),
                    kind: MergeActionKind::Restored,
                });
            }
            Some(live_item) => {
                if !values_equivalent(live_item, overlay) {
                    if mode == MergeMode::OverlayWins {
                        parent.insert(key, Item::Value(overlay_value.clone()));
                        actions.push(MergeAction {
                            path: render_segments(segments),
                            kind: MergeActionKind::Overwritten,
                        });
                    } else {
                        actions.push(MergeAction {
                            path: render_segments(segments),
                            kind: MergeActionKind::KeptLive,
                        });
                    }
                }
            }
        },
        Item::ArrayOfTables(overlay_array) => {
            let live_item = parent.get(key);
            match live_item {
                None => {
                    if !include_self {
                        return;
                    }
                    let item = Item::ArrayOfTables(overlay_array.clone());
                    record_subtree(&item, segments, actions);
                    parent.insert(key, item);
                }
                Some(existing) => {
                    if !values_equivalent(existing, overlay) {
                        if mode == MergeMode::OverlayWins {
                            let item = Item::ArrayOfTables(overlay_array.clone());
                            record_subtree(&item, segments, actions);
                            parent.insert(key, item);
                        } else {
                            actions.push(MergeAction {
                                path: render_segments(segments),
                                kind: MergeActionKind::KeptLive,
                            });
                        }
                    }
                }
            }
        }
    }
}

fn values_equivalent(live: &Item, overlay: &Item) -> bool {
    render_item(live) == render_item(overlay)
}

fn render_item(item: &Item) -> String {
    match item {
        Item::None => String::new(),
        Item::Value(value) => render_value(value),
        Item::Table(table) => {
            let mut parts: Vec<String> = Vec::new();
            for (key, child) in table.iter() {
                parts.push(format!("{key}={}", render_item(child)));
            }
            parts.sort();
            parts.join(",")
        }
        Item::ArrayOfTables(array) => array
            .iter()
            .map(|table| {
                let mut parts: Vec<String> = Vec::new();
                for (key, child) in table.iter() {
                    parts.push(format!("{key}={}", render_item(child)));
                }
                parts.sort();
                parts.join(",")
            })
            .collect::<Vec<_>>()
            .join(";"),
    }
}

fn record_subtree(item: &Item, segments: &[String], actions: &mut Vec<MergeAction>) {
    let mut leaves = BTreeMap::new();
    let mut path = segments.to_vec();
    walk(item, &mut path, None, &mut leaves);
    for leaf_path in leaves.keys() {
        actions.push(MergeAction {
            path: leaf_path.clone(),
            // Git Hub@OxygenAILab | Oxygen  AILab  @Star  sailsClo ver
            kind: MergeActionKind::Restored,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::Policy;

    fn parse(text: &str) -> DocumentMut {
        text.parse().unwrap()
    }

    fn compiled() -> CompiledPolicy {
        Policy::default().compile().unwrap()
    }

    #[test]
    fn projection_drops_managed_and_ignored_subtrees() {
        let doc = parse(
            r#"
model = "gpt-5"
model_provider = "custom"
approval_policy = "never"

[model_providers.custom]
base_url = "http://localhost:3000/v1"

[mcp_servers.node_repl]
command = "node_repl.exe"

[mcp_servers.prima-mock-api]
command = "pma.exe"

[plugins."pdf@openai-primary-runtime"]
enabled = true
"#,
        );
        let overlay = project(&doc, &compiled()).unwrap();
        let text = overlay.to_string();
        assert!(text.contains("approval_policy"));
        assert!(text.contains("prima-mock-api"));
        assert!(text.contains("plugins"));
        assert!(!text.contains("base_url"));
        assert!(!text.contains("node_repl"));
        // `model` is managed and unpinned: dropped. `model_provider` is managed
        // but pinned by default (sessions key on provider identity across
        // compaction), so the protected value must survive projection.
        assert!(!text.contains("\"gpt-5\""));
        assert!(text.contains("model_provider"));
    }

    #[test]
    fn overlay_restores_missing_and_keeps_managed_live() {
        let base = parse(
            r#"
model = "old-model"
approval_policy = "never"

[mcp_servers.prima-mock-api]
command = "pma.exe"

[desktop]
followUpQueueMode = "queue"
"#,
        );
        let policy = compiled();
        let overlay = project(&base, &policy).unwrap();
        let mut live = parse(
            r#"
model = "new-model"
model_provider = "SailsAPI"
"#,
        );
        let actions = apply_overlay(&mut live, &overlay, &policy, MergeMode::OverlayWins);
        assert!(actions
            .iter()
            .any(|action| action.path == "approval_policy"
                && action.kind == MergeActionKind::Restored));
        assert!(actions
            .iter()
            .any(|action| action.path == "mcp_servers.prima-mock-api.command"));
        let text = live.to_string();
        assert!(text.contains("new-model"));
        assert!(text.contains("approval_policy"));
        assert!(text.contains("pma.exe"));
        assert!(!text.contains("old-model"));
    }

    #[test]
    fn overlay_wins_on_conflict_unless_live_mode() {
        let base = parse("reasoning = \"max\"\n");
        let policy = compiled();
        let overlay = project(&base, &policy).unwrap();

        let mut live = parse("reasoning = \"low\"\n");
        apply_overlay(&mut live, &overlay, &policy, MergeMode::OverlayWins);
        assert_eq!(live["reasoning"].as_str(), Some("max"));

        let mut live = parse("reasoning = \"low\"\n");
        let actions = apply_overlay(&mut live, &overlay, &policy, MergeMode::LiveWins);
        assert_eq!(live["reasoning"].as_str(), Some("low"));
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, MergeActionKind::KeptLive);
        // GitHub@OxygenAILa b | Oxyg   e nAILab@StarsailsClover
    }

    #[test]
    fn leaf_collection_skips_empty_tables() {
        let doc = parse("[mcp_servers]\n");
        assert!(collect_protected_leaves(&doc, &compiled()).is_empty());
    }

    #[test]
    fn pinned_leaf_survives_projection_and_merge() {
        let policy = Policy {
            pinned: vec!["model_providers.SailsAPI.name".to_string()],
            ..Policy::default()
        };
        let policy = policy.compile().unwrap();
        let baseline = parse(
            r#"
model = "x"

[model_providers.SailsAPI]
base_url = "http://proxy"
name = "SailsAPI"
wire_api = "responses"

[plugins."pdf@openai-primary-runtime"]
enabled = true
"#,
        );
        let overlay = project(&baseline, &policy).unwrap();
        let overlay_text = overlay.to_string();
        assert!(overlay_text.contains("name = \"SailsAPI\""));
        assert!(!overlay_text.contains("base_url"));
        assert!(overlay_text.contains("plugins"));

        let mut live = parse(
            r#"
model = "x"

[model_providers.SailsAPI]
base_url = "http://proxy"
name = "OpenAI"
wire_api = "responses"
"#,
        );
        let actions = apply_overlay(&mut live, &overlay, &policy, MergeMode::OverlayWins);
        assert_eq!(
            live["model_providers"]["SailsAPI"]["name"].as_str(),
            Some("SailsAPI")
        );
        assert!(actions.iter().any(|action| {
            action.path == "model_providers.SailsAPI.name"
                && action.kind == MergeActionKind::Overwritten
        }));
        assert_eq!(
            live["model_providers"]["SailsAPI"]["base_url"].as_str(),
            Some("http://proxy")
        );
    }

    #[test]
    fn pinned_leaf_does_not_resurrect_a_missing_managed_parent() {
        let policy = Policy {
            pinned: vec!["model_providers.SailsAPI.name".to_string()],
            ..Policy::default()
        };
        let policy = policy.compile().unwrap();
        let baseline = parse("[model_providers.SailsAPI]\nname = \"SailsAPI\"\n");
        let overlay = project(&baseline, &policy).unwrap();
        let mut live = parse("model = \"x\"\n");
        apply_overlay(&mut live, &overlay, &policy, MergeMode::OverlayWins);
        assert!(live.get("model_providers").is_none());
    }
}
