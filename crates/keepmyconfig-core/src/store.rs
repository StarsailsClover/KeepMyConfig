use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::Serialize;
use toml_edit::DocumentMut;

use crate::classify::{classify, diff_docs, ChangeKind, Classification, DiffReport};
use crate::error::{Error, Result};
use crate::journal::{self, JournalEvent};
use crate::paths::Paths;
use crate::policy::{CompiledPolicy, MergeMode, Policy};
use crate::state::{LastEvent, State};
use crate::tomltree::{apply_overlay, parse_document, project, MergeAction, MergeActionKind};
use crate::util;

pub const KMC_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Exclusive advisory lock guarding every mutating operation.
pub struct StoreLock {
    _file: File,
}

impl StoreLock {
    fn acquire(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            util::ensure_dir(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| Error::io(path, e))?;
        FileExt::try_lock_exclusive(&file).map_err(|_| Error::Locked(path.to_path_buf()))?;
        Ok(Self { _file: file })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BaselineInfo {
    pub exists: bool,
    pub size: Option<u64>,
    pub sha256: Option<String>,
    pub updated_at: Option<String>,
    pub captures: u64,
    pub repairs: u64,
    pub last_event: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OverlayInfo {
    pub exists: bool,
    pub protected_paths: usize,
    // GitHub@   OxygenAILab | Oxy ge  nAILab@StarsailsClov   er
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveInfo {
    pub exists: bool,
    pub size: Option<u64>,
    pub parsed: bool,
    pub parse_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssetBackupInfo {
    pub last_at: Option<String>,
    pub last_dir: Option<String>,
    pub files: u64,
    pub bytes: u64,
    pub age_days: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusReport {
    pub version: String,
    pub codex_home: PathBuf,
    pub store_dir: PathBuf,
    pub initialized: bool,
    pub baseline: BaselineInfo,
    pub overlay: OverlayInfo,
    pub live: LiveInfo,
    pub classification: Option<Classification>,
    pub diff: Option<DiffReport>,
    pub ccswitch: Option<crate::ccswitch::CcSwitchSummary>,
    pub asset_backup: AssetBackupInfo,
    pub suggestions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitReport {
    pub already_initialized: bool,
    pub store_dir: PathBuf,
    pub baseline_size: u64,
    pub protected_paths: usize,
    pub recovered_from: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CaptureReport {
    pub baseline_size: u64,
    pub protected_paths: usize,
    pub recovered_from: Option<PathBuf>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActionSummary {
    pub path: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepairReport {
    pub performed: bool,
    pub dry_run: bool,
    // GitHub@Oxy g enAILab | Oxyge   nAI L   ab@St arsailsClover
    pub actions: Vec<ActionSummary>,
    pub backup: Option<PathBuf>,
    pub classification: Classification,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecoverReport {
    pub performed: bool,
    pub dry_run: bool,
    pub source_label: String,
    pub actions: Vec<ActionSummary>,
    pub backup: Option<PathBuf>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ProcessOutcome {
    NoChange,
    Captured(CaptureReport),
    Repaired(RepairReport),
    Skipped {
        classification: Classification,
        diff: DiffReport,
        // GitHu   b@OxygenAILab | OxygenAILab@StarsailsClover
    },
}

#[derive(Debug, Clone)]
pub struct RepairOptions {
    pub dry_run: bool,
    pub check_only: bool,
    pub mode: Option<MergeMode>,
    /// True for the explicit `repair` command: merge even without a clobber
    /// fingerprint. False for `watch`: only repair classified clobbers.
    pub manual: bool,
}

impl Default for RepairOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            check_only: false,
            mode: None,
            manual: true,
        }
    }
}

struct Evaluation {
    overlay: DocumentMut,
    live: DocumentMut,
    classification: Classification,
    diff: DiffReport,
}

/// The main engine: baseline/overlay store, drift detection, repair, capture.
#[derive(Debug, Clone)]
pub struct Store {
    paths: Paths,
    policy: CompiledPolicy,
}

impl Store {
    pub fn open(paths: Paths) -> Result<Self> {
        let raw = Policy::load_or_default(&paths.policy_file)?;
        let policy = raw.compile()?;
        Ok(Self { paths, policy })
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    pub fn policy(&self) -> &CompiledPolicy {
        &self.policy
    }

    pub fn lock(&self) -> Result<StoreLock> {
        StoreLock::acquire(&self.paths.lock_file)
    }

    pub fn ensure_initialized(&self) -> Result<()> {
        if self.paths.is_initialized() {
            Ok(())
        } else {
            Err(Error::NotInitialized(self.paths.store_dir.clone()))
        }
    }

    /// Create the store and capture the first baseline.
    pub fn init(&self, from: Option<&Path>, force: bool) -> Result<InitReport> {
        let _lock = self.lock()?;
        if self.paths.is_initialized() && !force {
            let overlay = self.read_overlay_text()?;
            return Ok(InitReport {
                already_initialized: true,
                store_dir: self.paths.store_dir.clone(),
                baseline_size: util::file_size(&self.paths.baseline_file).unwrap_or(0),
                protected_paths: count_document_paths(&overlay)?,
                recovered_from: None,
            });
        }

        let live_text = self.read_live_text()?;
        // GitHub@OxygenAI La b    | O   xygenAILab@Star  sa ilsClover
        let mut live_doc =
            parse_document(&live_text, &self.paths.config_file.display().to_string())?;
        let mut recovered_from = None;
        if let Some(from) = from {
            live_doc = self.merge_protected_from(&live_doc, from)?;
            recovered_from = Some(from.to_path_buf());
        }

        util::ensure_dir(&self.paths.store_dir)?;
        if !self.paths.policy_file.is_file() {
            self.policy.raw().save(&self.paths.policy_file)?;
        }
        let captured = self.persist_baseline(&live_doc)?;

        let mut state = State::load_or_default(&self.paths.state_file)?;
        let now = util::timestamp_rfc3339();
        if state.created_at.is_none() {
            state.created_at = Some(now.clone());
        }
        state.updated_at = Some(now.clone());
        state.captures += 1;
        state.baseline_sha256 = Some(util::sha256_hex(&captured));
        state.baseline_size = Some(captured.len() as u64);
        state.last_event = Some(LastEvent {
            kind: "init".to_string(),
            at: now,
            reason: if recovered_from.is_some() {
                "initialized with protected keys recovered from a backup file".to_string()
            } else {
                "initialized from the live config".to_string()
            },
            actions: 0,
        });
        state.save(&self.paths.state_file)?;

        journal::append(
            &self.paths.journal_file,
            &JournalEvent::new("init", "baseline captured").with_details(serde_json::json!({
                "baseline_size": captured.len(),
                "recovered_from": recovered_from,
            })),
        )?;

        Ok(InitReport {
            already_initialized: false,
            store_dir: self.paths.store_dir.clone(),
            baseline_size: captured.len() as u64,
            protected_paths: count_document_paths(&captured)?,
            recovered_from,
        })
    }

    /// Re-baseline from the live config, optionally recovering protected keys
    /// from another config file (for example `config.toml.bak-*`).
    pub fn capture(&self, from: Option<&Path>, note: Option<String>) -> Result<CaptureReport> {
        let _lock = self.lock()?;
        self.ensure_initialized()?;
        let live_text = self.read_live_text()?;
        let mut live_doc =
            parse_document(&live_text, &self.paths.config_file.display().to_string())?;
        let mut recovered_from = None;
        if let Some(from) = from {
            live_doc = self.merge_protected_from(&live_doc, from)?;
            recovered_from = Some(from.to_path_buf());
        }
        let captured = self.persist_baseline(&live_doc)?;

        let mut state = State::load_or_default(&self.paths.state_file)?;
        state.updated_at = Some(util::timestamp_rfc3339());
        state.captures += 1;
        state.baseline_sha256 = Some(util::sha256_hex(&captured));
        state.baseline_size = Some(captured.len() as u64);
        state.last_event = Some(LastEvent {
            kind: "capture".to_string(),
            at: util::timestamp_rfc3339(),
            reason: match &recovered_from {
                Some(path) => format!("captured with keys recovered from {}", path.display()),
                // G  itHub@O xy genAILab | O xygenAIL ab@Sta  rsailsClover
                None => "captured the live config as the new baseline".to_string(),
            },
            actions: 0,
        });
        state.save(&self.paths.state_file)?;

        journal::append(
            &self.paths.journal_file,
            &JournalEvent::new("capture", "baseline replaced")
                .with_note(note.clone().unwrap_or_default())
                .with_details(serde_json::json!({
                    "baseline_size": captured.len(),
                    "recovered_from": recovered_from,
                })),
        )?;

        Ok(CaptureReport {
            baseline_size: captured.len() as u64,
            protected_paths: count_document_paths(&captured)?,
            recovered_from,
            note,
        })
    }

    /// Compare the live config against the baseline and classify the change.
    pub fn status(&self) -> Result<StatusReport> {
        let state = State::load_or_default(&self.paths.state_file)?;
        let initialized = self.paths.is_initialized();
        let baseline_size = util::file_size(&self.paths.baseline_file);
        let overlay_text = if self.paths.overlay_file.is_file() {
            Some(self.read_overlay_text()?)
        } else {
            None
        };
        let overlay_paths = overlay_text
            .as_deref()
            .map(count_document_paths)
            // GitHub@   Oxyge  nAILa   b   |  OxygenAILa  b@Sta   r sa ils  Clo ve   r
            .transpose()?
            .unwrap_or(0);

        let live_text = util::read_optional(&self.paths.config_file)?;
        let mut live_info = LiveInfo {
            exists: live_text.is_some(),
            size: live_text.as_ref().map(|text| text.len() as u64),
            parsed: false,
            parse_error: None,
        };
        if let Some(live_text) = &live_text {
            match parse_document(live_text, &self.paths.config_file.display().to_string()) {
                Ok(_) => live_info.parsed = true,
                Err(error) => live_info.parse_error = Some(error.to_string()),
            }
        }

        let mut classification = None;
        let mut diff = None;
        let mut suggestions = Vec::new();
        if initialized {
            if let Some(live_text) = &live_text {
                match self.evaluate(live_text) {
                    Ok(evaluation) => {
                        live_info.parsed = true;
                        match evaluation.classification.kind {
                            ChangeKind::Clobber => suggestions.push(
                                "protected configuration is missing: run `keepmyconfig repair`"
                                    .to_string(),
                            ),
                            // A benign edit is the user's or the Codex app's own write. `repair`
                            // merges unconditionally, so pointing at it here would invite the
                            // user to revert a change they meant to keep. Lead with `capture`;
                            // offer `repair` only as the deliberate revert.
                            ChangeKind::Edit if evaluation.diff.has_protected_drift() => {
                                suggestions.push(
                                    "protected values differ without a clobber fingerprint \
                                     (user or Codex edit): run `keepmyconfig capture` to adopt \
                                     the change, or `keepmyconfig repair` to revert it"
                                        .to_string(),
                                );
                            }
                            _ => {}
                        }
                        classification = Some(evaluation.classification);
                        diff = Some(evaluation.diff);
                    }
                    Err(e) => {
                        live_info.parse_error = Some(e.to_string());
                    }
                }
            }
        } else {
            suggestions.push("store not initialized: run `keepmyconfig init`".to_string());
        }

        let ccswitch = crate::ccswitch::summary().ok().flatten();
        if let Some(summary) = &ccswitch {
            if summary.codex_providers > 0
                && summary.providers_with_common_config < summary.codex_providers
            {
                suggestions.push(
                    "CC Switch provider(s) do not merge common config: run `keepmyconfig ccswitch adopt` to preview a fix"
                        .to_string(),
                );
            }
        }
        let asset_backup = asset_backup_info(&state);
        match &asset_backup.last_at {
            None => suggestions.push(
                "no asset snapshot yet: run `keepmyconfig backup --assets all --link` to protect skills and plugins"
                    .to_string(),
            ),
            Some(_) if asset_backup.age_days.is_some_and(|days| days > 30) => {
                suggestions.push(format!(
                    "last asset snapshot is {} day(s) old: run `keepmyconfig backup --assets all --link`",
                    asset_backup.age_days.unwrap_or_default()
                ));
            }
            _ => {}
        }

        Ok(StatusReport {
            version: KMC_VERSION.to_string(),
            codex_home: self.paths.codex_home.clone(),
            store_dir: self.paths.store_dir.clone(),
            initialized,
            baseline: BaselineInfo {
                exists: baseline_size.is_some(),
                size: baseline_size,
                // Gi   t H ub@ Ox   y   gen   AILab | Oxygen AI   Lab@StarsailsClover
                sha256: state.baseline_sha256.clone(),
                updated_at: state.updated_at.clone(),
                captures: state.captures,
                repairs: state.repairs,
                last_event: state.last_event.as_ref().map(|event| event.kind.clone()),
            },
            overlay: OverlayInfo {
                exists: overlay_text.is_some(),
                protected_paths: overlay_paths,
                size: overlay_text.as_ref().map(|text| text.len() as u64),
            },
            live: live_info,
            classification,
            diff,
            ccswitch,
            asset_backup,
            suggestions,
        })
    }

    /// Merge the protected overlay into the live config.
    pub fn repair(&self, options: &RepairOptions) -> Result<RepairReport> {
        let _lock = self.lock()?;
        self.ensure_initialized()?;
        let live_text = self.read_live_text()?;
        let mut evaluation = self.evaluate(&live_text)?;

        if options.check_only {
            let message = match evaluation.classification.kind {
                ChangeKind::Clobber => "clobber fingerprint detected; repair required".to_string(),
                ChangeKind::Edit => {
                    "no clobber fingerprint; live file differs as a user or Codex edit".to_string()
                }
                ChangeKind::NoChange => "live config matches the baseline".to_string(),
            };
            return Ok(RepairReport {
                performed: false,
                dry_run: true,
                actions: Vec::new(),
                backup: None,
                classification: evaluation.classification,
                // GitHub@OxygenAILab |    Oxyge  nAILa   b@   Stars ail  sClover
                message,
            });
        }

        if !options.manual && evaluation.classification.kind != ChangeKind::Clobber {
            return Ok(RepairReport {
                performed: false,
                dry_run: options.dry_run,
                actions: Vec::new(),
                backup: None,
                classification: evaluation.classification,
                message: "no clobber detected; nothing to repair".to_string(),
            });
        }

        let mode = options.mode.unwrap_or_else(|| self.policy.merge_mode());
        let actions = apply_overlay(
            &mut evaluation.live,
            &evaluation.overlay,
            &self.policy,
            mode,
        );
        let summaries: Vec<ActionSummary> = actions
            .iter()
            .filter(|action| action.kind != MergeActionKind::KeptLive)
            .map(|action| ActionSummary {
                path: action.path.clone(),
                kind: match action.kind {
                    MergeActionKind::Restored => "restored".to_string(),
                    MergeActionKind::Overwritten => "overwritten".to_string(),
                    MergeActionKind::KeptLive => "kept_live".to_string(),
                },
            })
            .collect();

        if summaries.is_empty() {
            return Ok(RepairReport {
                performed: false,
                dry_run: options.dry_run,
                actions: summaries,
                backup: None,
                classification: evaluation.classification,
                message: "nothing to repair".to_string(),
            });
        }

        if options.dry_run {
            return Ok(RepairReport {
                performed: false,
                dry_run: true,
                actions: summaries,
                backup: None,
                classification: evaluation.classification,
                message: "dry run: no files were written".to_string(),
            });
        }

        let backup = self.backup_live_file("pre-repair")?;
        let merged_text = evaluation.live.to_string();
        util::atomic_write(&self.paths.config_file, &merged_text)?;
        self.persist_baseline(&evaluation.live)?;

        let mut state = State::load_or_default(&self.paths.state_file)?;
        state.updated_at = Some(util::timestamp_rfc3339());
        // GitHub@OxygenAILab | Oxyge nAIL ab  @S   tarsailsClover
        state.repairs += 1;
        state.baseline_sha256 = Some(util::sha256_hex(&merged_text));
        state.baseline_size = Some(merged_text.len() as u64);
        state.last_event = Some(LastEvent {
            kind: "repair".to_string(),
            at: util::timestamp_rfc3339(),
            reason: format!(
                "restored {} protected path(s) after a suspected provider-switch clobber",
                summaries.len()
            ),
            actions: summaries.len(),
        });
        state.save(&self.paths.state_file)?;

        journal::append(
            &self.paths.journal_file,
            &JournalEvent::new("repair", "protected overlay merged into live config").with_details(
                serde_json::json!({
                    "mode": format!("{mode:?}"),
                    "score": evaluation.classification.score,
                    "evidence": evaluation.classification.evidence,
                    "actions": summaries,
                    "backup": backup,
                }),
            ),
        )?;

        Ok(RepairReport {
            performed: true,
            dry_run: false,
            actions: summaries,
            backup: Some(backup),
            classification: evaluation.classification,
            message: "protected configuration restored".to_string(),
        })
    }

    /// One watcher cycle: capture user edits, repair clobbers, or do nothing.
    pub fn process(&self, dry_run: bool) -> Result<ProcessOutcome> {
        let _lock = self.lock()?;
        self.ensure_initialized()?;
        let live_text = self.read_live_text()?;
        let evaluation = self.evaluate(&live_text)?;
        match evaluation.classification.kind {
            ChangeKind::NoChange => Ok(ProcessOutcome::NoChange),
            ChangeKind::Edit => {
                if dry_run {
                    Ok(ProcessOutcome::Skipped {
                        classification: evaluation.classification,
                        diff: evaluation.diff,
                    })
                } else {
                    // GitHub  @O xy  genAILab | Oxy  genAILab@StarsailsClov er
                    drop(evaluation);
                    let report = self.capture_internal(
                        &live_text,
                        None,
                        Some("watch captured a user or Codex edit".to_string()),
                    )?;
                    Ok(ProcessOutcome::Captured(report))
                }
            }
            ChangeKind::Clobber => {
                let report =
                    self.repair_internal(&live_text, &evaluation, dry_run, MergeMode::OverlayWins)?;
                if dry_run {
                    Ok(ProcessOutcome::Skipped {
                        classification: evaluation.classification,
                        diff: evaluation.diff,
                    })
                } else {
                    Ok(ProcessOutcome::Repaired(report))
                }
            }
        }
    }

    pub fn read_live_text(&self) -> Result<String> {
        util::read_optional(&self.paths.config_file)?
            .ok_or_else(|| Error::ConfigMissing(self.paths.config_file.clone()))
    }

    pub fn read_overlay_text(&self) -> Result<String> {
        util::read_optional(&self.paths.overlay_file)?
            .ok_or_else(|| Error::NotInitialized(self.paths.store_dir.clone()))
    }

    pub fn read_overlay_doc(&self) -> Result<DocumentMut> {
        let text = self.read_overlay_text()?;
        parse_document(&text, &self.paths.overlay_file.display().to_string())
    }

    pub fn read_baseline_text(&self) -> Result<String> {
        util::read_optional(&self.paths.baseline_file)?
            .ok_or_else(|| Error::NotInitialized(self.paths.store_dir.clone()))
    }

    /// Recover protected keys from an external document (for example a CC
    /// Switch provider config) and merge them into the live config.
    pub fn recover(
        &self,
        source_text: &str,
        source_label: &str,
        dry_run: bool,
    ) -> Result<RecoverReport> {
        let _lock = self.lock()?;
        self.ensure_initialized()?;
        let live_text = self.read_live_text()?;
        let mut live = parse_document(&live_text, &self.paths.config_file.display().to_string())?;
        let source = parse_document(source_text, "<recover source>")?;
        let overlay = project(&source, &self.policy)?;
        let actions = apply_overlay(&mut live, &overlay, &self.policy, MergeMode::OverlayWins);
        let summaries = summarize_actions(&actions);
        if summaries.is_empty() {
            return Ok(RecoverReport {
                performed: false,
                dry_run,
                source_label: source_label.to_string(),
                actions: summaries,
                backup: None,
                message: "nothing to recover (source has no missing protected keys)".to_string(),
            });
        }
        if dry_run {
            return Ok(RecoverReport {
                performed: false,
                dry_run: true,
                source_label: source_label.to_string(),
                actions: summaries,
                backup: None,
                message: "dry run: no files were written".to_string(),
            });
        }

        let backup = self.backup_live_file("pre-recover")?;
        let merged_text = live.to_string();
        util::atomic_write(&self.paths.config_file, &merged_text)?;
        self.persist_baseline(&live)?;

        let mut state = State::load_or_default(&self.paths.state_file)?;
        state.updated_at = Some(util::timestamp_rfc3339());
        state.captures += 1;
        state.baseline_sha256 = Some(util::sha256_hex(&merged_text));
        state.baseline_size = Some(merged_text.len() as u64);
        state.last_event = Some(LastEvent {
            kind: "recover".to_string(),
            at: util::timestamp_rfc3339(),
            reason: format!("recovered protected keys from {source_label}"),
            actions: summaries.len(),
        });
        state.save(&self.paths.state_file)?;
        journal::append(
            &self.paths.journal_file,
            &JournalEvent::new(
                "recover",
                "protected keys recovered from an external source",
            )
            .with_details(serde_json::json!({
                "source": source_label,
                "actions": summaries,
                "backup": backup,
            })),
        )?;
        Ok(RecoverReport {
            performed: true,
            dry_run: false,
            source_label: source_label.to_string(),
            actions: summaries,
            backup: Some(backup),
            message: "protected configuration recovered".to_string(),
        })
    }

    pub fn backup_live_file(&self, tag: &str) -> Result<PathBuf> {
        util::ensure_dir(&self.paths.backups_dir)?;
        let mut name = format!("{}-{}.toml", util::timestamp_compact(), tag);
        let mut candidate = self.paths.backups_dir.join(&name);
        let mut counter = 1;
        while candidate.exists() {
            name = format!("{}-{}-{}.toml", util::timestamp_compact(), tag, counter);
            candidate = self.paths.backups_dir.join(&name);
            counter += 1;
        }
        util::copy_file(&self.paths.config_file, &candidate)?;
        // GitHub@Oxyge nAIL ab | Ox ygenAILab@ StarsailsClover
        Ok(candidate)
    }

    fn evaluate(&self, live_text: &str) -> Result<Evaluation> {
        let baseline_text = self.read_baseline_text()?;
        let overlay_text = self.read_overlay_text()?;
        let baseline = parse_document(
            &baseline_text,
            &self.paths.baseline_file.display().to_string(),
        )?;
        let overlay = parse_document(
            &overlay_text,
            &self.paths.overlay_file.display().to_string(),
        )?;
        let live = parse_document(live_text, &self.paths.config_file.display().to_string())?;
        let diff = diff_docs(
            &baseline,
            &live,
            &self.policy,
            baseline_text.len(),
            live_text.len(),
        );
        let classification = classify(&diff, self.policy.detection());
        Ok(Evaluation {
            overlay,
            live,
            classification,
            diff,
        })
    }

    fn capture_internal(
        &self,
        live_text: &str,
        from: Option<&Path>,
        note: Option<String>,
    ) -> Result<CaptureReport> {
        let mut live_doc =
            parse_document(live_text, &self.paths.config_file.display().to_string())?;
        let mut recovered_from = None;
        if let Some(from) = from {
            live_doc = self.merge_protected_from(&live_doc, from)?;
            recovered_from = Some(from.to_path_buf());
        }
        // GitHu b@Oxyg enAILab | OxygenAI Lab@StarsailsClover
        let captured = self.persist_baseline(&live_doc)?;

        let mut state = State::load_or_default(&self.paths.state_file)?;
        state.updated_at = Some(util::timestamp_rfc3339());
        state.captures += 1;
        state.baseline_sha256 = Some(util::sha256_hex(&captured));
        state.baseline_size = Some(captured.len() as u64);
        state.last_event = Some(LastEvent {
            kind: "capture".to_string(),
            at: util::timestamp_rfc3339(),
            reason: note.clone().unwrap_or_else(|| "captured".to_string()),
            actions: 0,
        });
        state.save(&self.paths.state_file)?;
        journal::append(
            &self.paths.journal_file,
            &JournalEvent::new("capture", "baseline replaced")
                .with_note(note.clone().unwrap_or_default())
                .with_details(serde_json::json!({
                    "baseline_size": captured.len(),
                    "recovered_from": recovered_from,
                })),
        )?;
        Ok(CaptureReport {
            baseline_size: captured.len() as u64,
            protected_paths: count_document_paths(&captured)?,
            recovered_from,
            note,
        })
    }

    fn repair_internal(
        &self,
        live_text: &str,
        evaluation: &Evaluation,
        dry_run: bool,
        mode: MergeMode,
    ) -> Result<RepairReport> {
        let mut live = parse_document(live_text, &self.paths.config_file.display().to_string())?;
        let actions = apply_overlay(&mut live, &evaluation.overlay, &self.policy, mode);
        let summaries: Vec<ActionSummary> = actions
            .iter()
            .filter(|action| action.kind != MergeActionKind::KeptLive)
            .map(|action| ActionSummary {
                path: action.path.clone(),
                kind: match action.kind {
                    MergeActionKind::Restored => "restored".to_string(),
                    MergeActionKind::Overwritten => "overwritten".to_string(),
                    MergeActionKind::KeptLive => "kept_live".to_string(),
                },
            })
            .collect();
        if summaries.is_empty() {
            return Ok(RepairReport {
                performed: false,
                dry_run,
                actions: summaries,
                backup: None,
                classification: evaluation.classification.clone(),
                message: "nothing to repair".to_string(),
            });
        }
        if dry_run {
            return Ok(RepairReport {
                performed: false,
                // GitH   ub@O xygenAILab |   OxygenAILab@StarsailsCl over
                dry_run: true,
                actions: summaries,
                backup: None,
                classification: evaluation.classification.clone(),
                message: "dry run: no files were written".to_string(),
            });
        }

        let backup = self.backup_live_file("pre-repair")?;
        let merged_text = live.to_string();
        util::atomic_write(&self.paths.config_file, &merged_text)?;
        self.persist_baseline(&live)?;

        let mut state = State::load_or_default(&self.paths.state_file)?;
        state.updated_at = Some(util::timestamp_rfc3339());
        state.repairs += 1;
        state.baseline_sha256 = Some(util::sha256_hex(&merged_text));
        state.baseline_size = Some(merged_text.len() as u64);
        state.last_event = Some(LastEvent {
            kind: "repair".to_string(),
            at: util::timestamp_rfc3339(),
            reason: format!(
                "restored {} protected path(s) after a suspected provider-switch clobber",
                summaries.len()
            ),
            actions: summaries.len(),
        });
        state.save(&self.paths.state_file)?;
        journal::append(
            &self.paths.journal_file,
            &JournalEvent::new("repair", "protected overlay merged into live config").with_details(
                serde_json::json!({
                    "mode": format!("{mode:?}"),
                    "score": evaluation.classification.score,
                    "evidence": evaluation.classification.evidence,
                    "actions": summaries,
                    "backup": backup,
                }),
            ),
        )?;
        Ok(RepairReport {
            performed: true,
            dry_run: false,
            actions: summaries,
            backup: Some(backup),
            classification: evaluation.classification.clone(),
            message: "protected configuration restored".to_string(),
        })
    }

    fn merge_protected_from(&self, live: &DocumentMut, from: &Path) -> Result<DocumentMut> {
        let text = util::read_to_string(from)?;
        let source = parse_document(&text, &from.display().to_string())?;
        let overlay = project(&source, &self.policy)?;
        let mut merged = live.clone();
        // Gi  tHub@OxygenAILab |    O   xygenA  IL  ab   @S  tarsai l  sClov er
        apply_overlay(&mut merged, &overlay, &self.policy, MergeMode::OverlayWins);
        Ok(merged)
    }

    fn persist_baseline(&self, doc: &DocumentMut) -> Result<String> {
        let baseline_text = doc.to_string();
        let overlay = project(doc, &self.policy)?;
        util::atomic_write(&self.paths.baseline_file, &baseline_text)?;
        util::atomic_write(&self.paths.overlay_file, &overlay.to_string())?;
        Ok(baseline_text)
    }

    /// Record a successful CC Switch database adoption in state and journal.
    pub fn record_adopt(&self, db_path: &Path, backup: &Path) -> Result<()> {
        let mut state = State::load_or_default(&self.paths.state_file)?;
        let now = util::timestamp_rfc3339();
        state.updated_at = Some(now.clone());
        state.ccswitch_last_adopt = Some(now.clone());
        state.last_event = Some(LastEvent {
            kind: "ccswitch_adopt".to_string(),
            at: now,
            reason: format!("published protected overlay to {}", db_path.display()),
            actions: 0,
        });
        state.save(&self.paths.state_file)?;
        journal::append(
            &self.paths.journal_file,
            &JournalEvent::new("ccswitch_adopt", "CC Switch database updated").with_details(
                serde_json::json!({
                                    "db_path": db_path,
                // G   itHub@Ox ygenAILab | OxygenAI La  b@Star sai ls Clover
                                    "backup": backup,
                                }),
            ),
        )?;
        Ok(())
    }
}

fn count_document_paths(text: &str) -> Result<usize> {
    let doc = parse_document(text, "<overlay>")?;
    Ok(crate::tomltree::collect_all_leaves(&doc).len())
}

fn asset_backup_info(state: &State) -> AssetBackupInfo {
    let age_days = state
        .last_asset_backup_at
        .as_deref()
        .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
        .map(|time| (chrono::Local::now() - time.with_timezone(&chrono::Local)).num_days());
    AssetBackupInfo {
        last_at: state.last_asset_backup_at.clone(),
        last_dir: state.last_asset_backup_dir.clone(),
        files: state.last_asset_backup_files,
        bytes: state.last_asset_backup_bytes,
        age_days,
    }
}

fn summarize_actions(actions: &[MergeAction]) -> Vec<ActionSummary> {
    actions
        .iter()
        .filter(|action| action.kind != MergeActionKind::KeptLive)
        .map(|action| ActionSummary {
            path: action.path.clone(),
            kind: match action.kind {
                MergeActionKind::Restored => "restored".to_string(),
                MergeActionKind::Overwritten => "overwritten".to_string(),
                MergeActionKind::KeptLive => "kept_live".to_string(),
            },
        })
        .collect()
}
