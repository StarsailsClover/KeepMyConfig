mod autostart;
mod bus;
mod render;
mod rules;
mod sessions_render;

/// Ops is always watched: it is the shared mailbox every session can write to.
const DEFAULT_BUS_WATCH: &str = "env-ops=Ops";

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use keepmyconfig_core::assets::{self, BackupOptions, RestoreOptions};
use keepmyconfig_core::ccswitch::{self, AdoptOptions};
use keepmyconfig_core::paths::Paths;
use keepmyconfig_core::sessions::TrimOptions as SessionTrimOptions;
use keepmyconfig_core::sessions::{self, RepairOptions as SessionRepairOptions};
use keepmyconfig_core::store::{RepairOptions, Store};
use keepmyconfig_core::watch::{self, WatchOptions};
use serde::Serialize;

#[derive(Parser, Debug)]
#[command(
    name = "keepmyconfig",
    version = keepmyconfig_core::KMC_VERSION,
    about = "Keep user-owned Codex configuration, MCP servers, skills, and plugins alive across CC Switch provider switches and restarts.",
    long_about = None
)]
struct Cli {
    /// Codex home directory (default: $CODEX_HOME or ~/.codex)
    #[arg(long, global = true, value_name = "PATH", env = "CODEX_HOME")]
    codex_home: Option<PathBuf>,

    /// Store directory (default: <codex_home>/.keepmyconfig)
    #[arg(long, global = true, value_name = "PATH")]
    store: Option<PathBuf>,

    /// Cowork root for the collaboration bus (default: ~/Documents/Cowork)
    #[arg(long, global = true, value_name = "PATH", env = "KMC_COWORK")]
    cowork: Option<PathBuf>,

    /// Machine-readable JSON output
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Create the store and capture the first baseline
    Init {
        // G  it  Hub@Ox ygenAILa  b | Oxygen   AILa b@   StarsailsClover
        /// Recover protected keys from a backup file (for example config.toml.bak-*)
        #[arg(long, value_name = "FILE")]
        from: Option<PathBuf>,
        /// Re-capture even if the store is already initialized
        #[arg(long)]
        force: bool,
    },

    /// Re-baseline after intentional edits, optionally recovering keys from a file
    Capture {
        #[arg(long, value_name = "FILE")]
        from: Option<PathBuf>,
        #[arg(long)]
        note: Option<String>,
    },

    /// Show drift, protection health, and CC Switch state
    Status {
        /// Exit 2 when protected drift is detected
        #[arg(long)]
        check: bool,
    },

    /// Show a path-level diff between the baseline and the live config
    Diff {
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Exit 2 when protected drift is detected
        #[arg(long)]
        check: bool,
    },

    /// Merge the protected overlay into the live config now
    Repair {
        #[arg(long)]
        dry_run: bool,
        /// Report only; exit 2 when a clobber fingerprint is present
        #[arg(long)]
        check: bool,
        // GitHub@OxygenAI   L   ab | O  xy   genAILa  b@St arsailsClover
        /// Prefer the live file on conflicting protected paths
        #[arg(long)]
        prefer_live: bool,
        /// Prefer the overlay on conflicting protected paths (default)
        #[arg(long)]
        prefer_overlay: bool,
    },

    /// Watch the config and repair CC Switch clobbers automatically
    Watch {
        /// Run one detection cycle and exit
        #[arg(long)]
        once: bool,
        /// Detect and report without writing
        #[arg(long)]
        dry_run: bool,
        /// Native-watcher backstop poll interval
        #[arg(long, default_value = "30s")]
        poll: String,
        /// Wait this long after the last write before processing
        #[arg(long, default_value = "800ms")]
        debounce: String,
        /// Suppress per-event output
        #[arg(long)]
        quiet: bool,
    },

    /// Back up configuration assets (config, skills, plugins, prompts, rules)
    Backup {
        #[arg(long, value_delimiter = ',', default_value = "config")]
        assets: Vec<String>,
        /// Hard-link files instead of copying (fast, lower disk use)
        #[arg(long)]
        // GitHub@O   xyg enAIL  ab  |  Oxyge   nAILab@StarsailsClover
        link: bool,
        /// Include plugins/cache/**
        #[arg(long)]
        include_cache: bool,
    },

    /// Restore missing files from an asset backup
    RestoreAssets {
        #[arg(long, value_name = "DIR")]
        from: Option<PathBuf>,
        #[arg(long)]
        overwrite: bool,
        #[arg(long)]
        dry_run: bool,
        /// Also restore auth.json and other backup-only files
        #[arg(long)]
        include_backup_only: bool,
    },

    /// Inspect or update CC Switch's own provider/MCP database (opt-in)
    Ccswitch {
        #[command(subcommand)]
        command: CcSwitchCommand,
    },

    /// Find and repair unpaired tool records in Codex session rollouts
    Sessions {
        #[command(subcommand)]
        command: SessionsCommand,
    },

    /// Cross-session collaboration bus (replaces the Codex in-app message system)
    Bus {
        #[command(subcommand)]
        command: BusCommand,
    },

    /// Print the recommended AGENTS.md collaboration rule block
    Rules {
        /// Language: zh (default) or en
        #[arg(long, default_value = "zh")]
        lang: String,
    },

    /// Check the environment and report compatibility
    Doctor,

    /// Install or inspect logon/periodic self-healing tasks (Windows)
    Autostart {
        #[command(subcommand)]
        command: AutostartCommand,
    },
}

#[derive(Subcommand, Debug)]
enum CcSwitchCommand {
    /// Show the current provider/common-config/MCP state
    Inspect,
    /// Publish the protected overlay into CC Switch's database
    Adopt {
        #[arg(long, value_name = "FILE")]
        db: Option<PathBuf>,
        /// Write changes (default is a dry run)
        #[arg(long)]
        apply: bool,
        /// Allow writing while CC Switch is running
        #[arg(long)]
        force: bool,
        /// Do not register protected MCP servers in CC Switch
        #[arg(long)]
        no_mcp: bool,
    },
    /// List database snapshots taken before adopt/restore
    Backups,
    /// Restore a database snapshot
    Restore {
        #[arg(long, value_name = "FILE")]
        backup: PathBuf,
        #[arg(long)]
        force: bool,
    },
    /// Recover protected configuration from a stored provider config
    Recover {
        #[arg(long, value_name = "FILE")]
        db: Option<PathBuf>,
        /// Provider id or name (defaults to the config with the most protected paths)
        #[arg(long)]
        provider: Option<String>,
        /// Write changes (default is a dry run)
        #[arg(long)]
        apply: bool,
    },
}

#[derive(Subcommand, Debug)]
enum AutostartCommand {
    /// Install a logon daemon plus a periodic one-shot repair task
    Install {
        /// Periodic repair interval in minutes
        #[arg(long, default_value_t = 5)]
        interval_minutes: u64,
        /// Only install the periodic check (no long-running daemon)
        #[arg(long)]
        no_daemon: bool,
        /// Installation mechanism: Task Scheduler, Startup folder, or auto-fallback
        #[arg(long, value_enum, default_value_t = AutostartMethod::Auto)]
        method: AutostartMethod,
        /// Watch a bus channel for an actor, `CHANNEL=ACTOR` (repeatable)
        #[arg(long = "bus", value_name = "CHANNEL=ACTOR")]
        bus: Vec<String>,
    },
    /// Remove the scheduled tasks and wrapper scripts
    Uninstall,
    /// Show whether the tasks are installed
    Status,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum AutostartMethod {
    Auto,
    Task,
    Startup,
}

#[derive(Subcommand, Debug)]
enum SessionsCommand {
    /// Report unpaired tool records across every session rollout
    Scan {
        /// Include the full per-file finding list
        #[arg(long)]
        full: bool,
    },
    /// Remove unpaired tool records (dry run unless --apply)
    Repair {
        /// Write the changes
        #[arg(long)]
        apply: bool,
        /// Which records to remove: outputs (default), calls, or all
        #[arg(long, default_value = "outputs")]
        what: String,
        /// Also delete the orphan rows from the history database projection
        #[arg(long)]
        purge_projection: bool,
        /// Allow writing the history database while Codex is running
        #[arg(long)]
        force: bool,
    },
    /// Replace inline base64 images with a path-only placeholder (dry run unless --apply)
    TrimImages {
        /// Write the changes
        #[arg(long)]
        apply: bool,
        /// Only this thread id
        #[arg(long)]
        thread: Option<String>,
        /// Also delete the trimmed rows from the history database projection
        #[arg(long)]
        purge_projection: bool,
        /// Allow writing the history database while Codex is running
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand, Debug)]
enum BusCommand {
    /// Create the channel board, index, and README
    Init {
        #[arg(long, default_value = "default")]
        channel: String,
    },
    /// Backfill the index from an existing hand-written board (board is unchanged)
    Adopt {
        #[arg(long, default_value = "default")]
        channel: String,
    },
    /// Append a message to a channel
    Post {
        #[arg(long, default_value = "default")]
        channel: String,
        /// Speaker, for example "UI (UI9)"
        #[arg(long)]
        from: String,
        /// Recipient names (comma separated, or "all")
        #[arg(long, value_delimiter = ',')]
        to: Vec<String>,
        /// Message body
        #[arg(long)]
        body: String,
        /// Reference another message id
        #[arg(long)]
        reply_to: Option<String>,
        /// Question tags such as Q1
        #[arg(long, value_delimiter = ',')]
        ask: Vec<String>,
        /// Thread file under threads/
        #[arg(long)]
        thread: Option<String>,
        /// Mark the message as expecting an answer
        #[arg(long)]
        wants_reply: bool,
        /// Override the timestamp
        #[arg(long)]
        ts: Option<String>,
    },
    /// Show a channel window and advance the reader cursor
    Read {
        #[arg(long, default_value = "default")]
        channel: String,
        /// Actor whose cursor is used, for example "UI (UI9)"
        #[arg(long)]
        r#as: Option<String>,
        /// Only messages after this id
        #[arg(long)]
        since: Option<String>,
        /// Only messages addressed to this recipient
        #[arg(long)]
        to: Option<String>,
        /// Selector (only "all" is supported today)
        #[arg(long, default_value = "all")]
        selector: String,
        /// Shorthand for --selector unread
        #[arg(long)]
        unread: bool,
        /// Print full bodies instead of previews
        #[arg(long)]
        full: bool,
        /// Print without advancing the reader cursor
        #[arg(long)]
        peek: bool,
    },
    /// Per-recipient unread counts
    Unread {
        #[arg(long, default_value = "default")]
        channel: String,
    },
    /// Show a desktop toast for the acting session (does not wake it)
    Popup {
        #[arg(long, default_value = "default")]
        channel: String,
        #[arg(long)]
        r#as: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Notify-only watch: toast new mail for one actor, never wake a session
    Watch {
        #[arg(long, default_value = "default")]
        channel: String,
        #[arg(long)]
        r#as: String,
        /// Run one pass and exit
        #[arg(long)]
        once: bool,
        /// Detect and report without raising a toast
        #[arg(long)]
        dry_run: bool,
        /// Poll interval for the loop mode (for example 60s)
        #[arg(long, default_value = "60s")]
        poll: String,
        /// Suppress the report line
        #[arg(long)]
        quiet: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    // Gi   tHub@OxygenAILab  | OxygenAILab @   StarsailsClover
    match run(cli) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(1)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    let paths = Paths::resolve(cli.codex_home, cli.store)?;
    let store = Store::open(paths)?;
    let json = cli.json;

    match cli.command {
        Command::Init { from, force } => {
            let report = store
                .init(from.as_deref(), force)
                .context("cannot initialize the store")?;
            print_value(json, &report, render::init_text(&report))?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Capture { from, note } => {
            let report = store
                .capture(from.as_deref(), note)
                .context("cannot capture the baseline")?;
            print_value(json, &report, render::capture_text(&report))?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Status { check } => {
            let report = store.status().context("cannot read status")?;
            print_value(json, &report, render::status_text(&report))?;
            let drift = report
                .diff
                .as_ref()
                .is_some_and(|diff| diff.has_protected_drift())
                || report
                    .classification
                    .as_ref()
                    .is_some_and(|classification| {
                        classification.kind == keepmyconfig_core::classify::ChangeKind::Clobber
                    });
            if check && drift {
                Ok(ExitCode::from(2))
            } else {
                Ok(ExitCode::SUCCESS)
            }
        }
        Command::Diff { limit, check } => {
            let report = store.status().context("cannot read diff")?;
            if json {
                print_json(&report)?;
            } else {
                print!("{}", render::diff_text(&report, limit));
            }
            let drift = report
                .diff
                .as_ref()
                .is_some_and(|diff| diff.has_protected_drift());
            if check && drift {
                Ok(ExitCode::from(2))
            } else {
                Ok(ExitCode::SUCCESS)
            }
        }
        Command::Repair {
            dry_run,
            check,
            prefer_live,
            prefer_overlay,
        } => {
            if prefer_live && prefer_overlay {
                anyhow::bail!("--prefer-live and --prefer-overlay are mutually exclusive");
            }
            let options = RepairOptions {
                dry_run,
                // GitHub@OxygenAILab | Ox   ygen A   ILab@StarsailsClover
                check_only: check,
                mode: if prefer_live {
                    Some(keepmyconfig_core::MergeMode::LiveWins)
                } else if prefer_overlay {
                    Some(keepmyconfig_core::MergeMode::OverlayWins)
                } else {
                    None
                },
                manual: true,
            };
            let report = store.repair(&options).context("cannot repair the config")?;
            print_value(json, &report, render::repair_text(&report))?;
            if check
                && report.classification.kind == keepmyconfig_core::classify::ChangeKind::Clobber
            {
                Ok(ExitCode::from(2))
            } else {
                Ok(ExitCode::SUCCESS)
            }
        }
        Command::Watch {
            once,
            dry_run,
            poll,
            debounce,
            quiet,
        } => {
            let options = WatchOptions {
                once,
                dry_run,
                debounce: parse_duration(&debounce)?,
                poll: parse_duration(&poll)?,
            };
            let mut printed_started = false;
            let report = watch::run(&store, &options, |event| {
                if quiet || json {
                    // GitHub@OxygenAILab | OxygenAILab@Stars   ailsClov   er
                    return;
                }
                if !printed_started {
                    if !options.once {
                        eprintln!("Press Ctrl+C to stop.");
                    }
                    printed_started = true;
                }
                println!("{}", render::watch_text(event));
            })
            .context("watch loop failed")?;
            if json {
                print_json(&report)?;
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Backup {
            assets,
            link,
            include_cache,
        } => {
            let options = BackupOptions {
                classes: assets,
                link,
                include_cache,
            };
            let report = assets::backup_assets(&store, &options).context("asset backup failed")?;
            print_value(json, &report, render::backup_text(&report))?;
            Ok(ExitCode::SUCCESS)
        }
        Command::RestoreAssets {
            from,
            overwrite,
            dry_run,
            include_backup_only,
        } => {
            let options = RestoreOptions {
                from,
                overwrite,
                dry_run,
                include_backup_only,
            };
            let report =
                assets::restore_assets(&store, &options).context("asset restore failed")?;
            print_value(json, &report, render::restore_text(&report))?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Ccswitch { command } => run_ccswitch(&store, command, json),
        Command::Sessions { command } => run_sessions(&store, command, json),
        Command::Bus { command } => run_bus(cli.cowork, command, json),
        Command::Rules { lang } => {
            print!("{}", rules::text(&lang));
            Ok(ExitCode::SUCCESS)
        }
        Command::Doctor => run_doctor(&store, json),
        Command::Autostart { command } => run_autostart(cli.cowork, &store, command, json),
    }
}

fn run_autostart(
    cowork: Option<PathBuf>,
    store: &Store,
    command: AutostartCommand,
    json: bool,
) -> Result<ExitCode> {
    match command {
        AutostartCommand::Install {
            interval_minutes,
            no_daemon,
            method,
            bus,
        } => {
            let method = match method {
                AutostartMethod::Auto => autostart::InstallMethod::Auto,
                AutostartMethod::Task => autostart::InstallMethod::Task,
                AutostartMethod::Startup => autostart::InstallMethod::Startup,
            };
            let mut watches = vec![DEFAULT_BUS_WATCH.to_string()];
            watches.extend(bus);
            let watches = parse_bus_watches(&watches)?;
            let cowork_root = bus::root(cowork)?;
            let report = autostart::install(
                store,
                interval_minutes,
                no_daemon,
                method,
                &watches,
                &cowork_root,
            )
            .context("cannot install autostart tasks")?;
            print_value(json, &report, render::autostart_text(&report))?;
        }
        AutostartCommand::Uninstall => {
            let report = autostart::uninstall(store).context("cannot remove autostart tasks")?;
            print_value(json, &report, render::autostart_text(&report))?;
        }
        AutostartCommand::Status => {
            let status = autostart::status(store).context("cannot read autostart status")?;
            print_value(json, &status, render::autostart_status_text(&status))?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Parse `CHANNEL=ACTOR` pairs for `autostart install --bus`.
fn parse_bus_watches(values: &[String]) -> Result<Vec<autostart::BusWatch>> {
    let mut watches = Vec::new();
    for value in values {
        let (channel, actor) = value
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("--bus expects CHANNEL=ACTOR, got '{value}'"))?;
        let channel = channel.trim();
        let actor = actor.trim();
        if channel.is_empty() || actor.is_empty() {
            anyhow::bail!("--bus expects CHANNEL=ACTOR, got '{value}'");
        }
        watches.push(autostart::BusWatch {
            channel: channel.to_string(),
            actor: actor.to_string(),
        });
    }
    Ok(watches)
}

fn run_sessions(store: &Store, command: SessionsCommand, json: bool) -> Result<ExitCode> {
    match command {
        SessionsCommand::Scan { full } => {
            let report = sessions::scan(store.paths().codex_home.as_path())
                .context("cannot scan Codex sessions")?;
            if json {
                print_json(&report)?;
            } else {
                print!("{}", sessions_render::scan_text(&report, full));
            }
            if report.orphan_outputs > 0 {
                Ok(ExitCode::from(2))
            } else {
                Ok(ExitCode::SUCCESS)
            }
        }
        SessionsCommand::Repair {
            apply,
            what,
            purge_projection,
            force,
        } => {
            let options = SessionRepairOptions {
                apply,
                what,
                purge_projection,
                force,
            };
            let report =
                sessions::repair(store, &options).context("cannot repair Codex sessions")?;
            print_value(json, &report, sessions_render::repair_text(&report))?;
            Ok(ExitCode::SUCCESS)
        }
        SessionsCommand::TrimImages {
            apply,
            thread,
            purge_projection,
            force,
        } => {
            let options = SessionTrimOptions {
                apply,
                thread,
                purge_projection,
                force,
            };
            let report = sessions::trim_images(store, &options)
                .context("cannot trim Codex session images")?;
            print_value(json, &report, sessions_render::trim_text(&report))?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn run_bus(cowork: Option<PathBuf>, command: BusCommand, json: bool) -> Result<ExitCode> {
    let root = bus::root(cowork)?;
    match command {
        BusCommand::Init { channel } => {
            let dir = bus::init(&root, &channel)?;
            if json {
                print_json(&serde_json::json!({ "channel": channel, "dir": dir }))?;
            } else {
                println!("已初始化协作总线频道 {channel}\n  {}", dir.display());
            }
        }
        BusCommand::Adopt { channel } => {
            let report = bus::adopt(&root, &channel)?;
            if json {
                print_json(&report)?;
            } else {
                println!(
                    "已接入频道 {}：回填 {} 条，跳过 {} 条（重复）",
                    report.channel, report.adopted, report.skipped
                );
                println!("  看板(未改动): {}", report.board);
                println!("  索引: {}", report.index);
                if !report.seeded_actors.is_empty() {
                    println!(
                        "  已把以下会话的游标置到末条(不回灌未读): {}",
                        report.seeded_actors.join(", ")
                    );
                }
            }
        }
        BusCommand::Post {
            channel,
            from,
            to,
            body,
            reply_to,
            ask,
            thread,
            wants_reply,
            ts,
        } => {
            let message = bus::Message {
                id: String::new(),
                channel: channel.clone(),
                ts: ts.unwrap_or_default(),
                from,
                to,
                body,
                reply_to,
                asks: ask,
                thread,
                wants_reply,
            };
            let report = bus::post(&root, message)?;
            if json {
                print_json(&report)?;
            } else if let Some(original) = &report.duplicate_of {
                println!("重复消息，已存在 {}（未重复写入）", original);
            } else {
                println!("已投递 {} → {}", report.id, report.recipients.join(", "));
                println!("  {}", report.board);
            }
        }
        BusCommand::Read {
            channel,
            r#as,
            since,
            to,
            selector,
            unread,
            full,
            peek,
        } => {
            let options = bus::ReadOptions {
                since,
                to_actor: to,
                selector: if unread {
                    "unread".to_string()
                } else {
                    selector
                },
                as_session: r#as,
                all: false,
                mark_read: !peek,
            };
            let report = bus::read(&root, &channel, &options)?;
            if json {
                print_json(&report)?;
            } else {
                print!("{}", bus::render_read(&report, full));
            }
        }
        BusCommand::Unread { channel } => {
            let counts = bus::unread(&root, &channel)?;
            if json {
                print_json(&counts)?;
            } else {
                print!("{}", bus::render_unread(&channel, &counts));
            }
        }
        BusCommand::Popup {
            channel,
            r#as,
            dry_run,
        } => {
            let text = bus::notify(&root, &channel, &r#as)?;
            if text.is_empty() {
                if !json {
                    println!("协作总线 {}: 没有未读。", r#as);
                }
            } else if dry_run {
                println!("{text}");
            } else {
                bus::popup("协作总线", &text)?;
                if !json {
                    println!("{text}");
                }
            }
        }
        BusCommand::Watch {
            channel,
            r#as,
            once,
            dry_run,
            poll,
            quiet,
        } => {
            let interval = parse_duration(&poll)?;
            loop {
                match bus::watch_once(&root, &channel, &r#as, !dry_run) {
                    Ok(report) => {
                        if json {
                            print_json(&report)?;
                        } else if !quiet && (report.unread > 0 || report.open_questions > 0) {
                            println!(
                                "[{}] {} 未读 {} · 待回复 {}",
                                report.channel, report.actor, report.unread, report.open_questions
                            );
                        }
                    }
                    // A one-shot pass should surface the error; a loop should keep
                    // waiting rather than die on a transient failure.
                    Err(error) if once => return Err(error),
                    Err(error) => eprintln!("bus watch: {error:#}"),
                }
                if once {
                    break;
                }
                std::thread::sleep(interval);
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn run_ccswitch(store: &Store, command: CcSwitchCommand, json: bool) -> Result<ExitCode> {
    match command {
        CcSwitchCommand::Inspect => {
            match ccswitch::summary().context("cannot inspect CC Switch")? {
                Some(summary) => {
                    if json {
                        print_json(&summary)?;
                    } else {
                        print!("{}", render::ccswitch_text(&summary));
                        println!(
                        "Run `keepmyconfig ccswitch adopt` to preview publishing the protected overlay."
                    );
                    }
                }
                None => {
                    println!("CC Switch database not found (~/.cc-switch/cc-switch.db).");
                }
            }
        }
        CcSwitchCommand::Adopt {
            db,
            apply,
            force,
            no_mcp,
        } => {
            let options = AdoptOptions {
                db_path: db,
                apply,
                force,
                include_mcp: !no_mcp,
                // GitH   ub@OxygenAILab | OxygenAILab@StarsailsCl  over
            };
            let report = ccswitch::adopt(store, &options).context("CC Switch adopt failed")?;
            print_value(json, &report, render::adopt_text(&report))?;
        }
        CcSwitchCommand::Backups => {
            let backups = ccswitch::list_database_backups(store)?;
            if json {
                print_json(&backups)?;
            } else if backups.is_empty() {
                println!("No CC Switch database backups found.");
            } else {
                for backup in backups {
                    println!("{}", backup.display());
                }
            }
        }
        CcSwitchCommand::Restore { backup, force } => {
            let report = ccswitch::restore_database(store, &backup, force)
                .context("CC Switch restore failed")?;
            print_value(json, &report, render::restore_db_text(&report))?;
        }
        CcSwitchCommand::Recover {
            db,
            provider,
            apply,
        } => {
            let report = ccswitch::recover(store, db, provider.as_deref(), apply)
                .context("CC Switch recovery failed")?;
            print_value(json, &report, render::ccswitch_recover_text(&report))?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[derive(Serialize)]
struct DoctorReport {
    version: String,
    codex_home: String,
    store_dir: String,
    checks: Vec<DoctorCheck>,
}

#[derive(Serialize)]
struct DoctorCheck {
    name: String,
    status: String,
    detail: String,
}

fn run_doctor(store: &Store, json: bool) -> Result<ExitCode> {
    let status = store.status().context("cannot read status")?;
    let mut checks = Vec::new();
    checks.push(check(
        "codex_home",
        status.codex_home.is_dir(),
        format!("{}", status.codex_home.display()),
    ));
    checks.push(check(
        "config_toml",
        status.live.exists && status.live.parsed,
        match (&status.live.parse_error, status.live.exists) {
            (Some(error), _) => error.clone(),
            (None, true) => "present and parseable".to_string(),
            (None, false) => "missing".to_string(),
        },
        // GitHub@OxygenAILab  | OxygenAILab@StarsailsClover
    ));
    checks.push(check(
        "store",
        status.initialized,
        if status.initialized {
            format!(
                "{} protected path(s) across {} capture(s)",
                status.overlay.protected_paths, status.baseline.captures
            )
        } else {
            "run `keepmyconfig init`".to_string()
        },
    ));
    let clobber = status
        .classification
        .as_ref()
        .is_some_and(|classification| {
            classification.kind == keepmyconfig_core::classify::ChangeKind::Clobber
        });
    checks.push(check(
        "protection",
        !clobber,
        if clobber {
            // GitHub@OxygenAILab | OxygenAI   Lab@Starsai  lsClove   r
            "protected drift detected; run `keepmyconfig repair`".to_string()
        } else if status
            .diff
            .as_ref()
            .is_some_and(|diff| diff.has_protected_drift())
        {
            // Drift without a clobber fingerprint is a user or Codex edit. `repair` merges
            // unconditionally, so name `capture` first and let the user choose.
            "no clobber fingerprint · protected values differ from the baseline; run `capture` to adopt or `repair` to revert".to_string()
        } else {
            "no clobber fingerprint".to_string()
        },
    ));
    let assets_ok = status.asset_backup.last_at.is_some()
        && status.asset_backup.age_days.is_some_and(|days| days <= 30);
    checks.push(check_status(
        "assets",
        if assets_ok { "ok" } else { "warn" },
        match &status.asset_backup.last_at {
            Some(at) => format!(
                "last snapshot {at} ({} files, {} bytes)",
                status.asset_backup.files, status.asset_backup.bytes
            ),
            None => "no snapshot yet (run `keepmyconfig backup --assets all --link`)".to_string(),
        },
    ));
    match ccswitch::summary() {
        Ok(Some(summary)) => {
            let healthy = summary.codex_providers == 0
                || summary.providers_with_common_config == summary.codex_providers;
            checks.push(check_status(
                "cc_switch",
                if healthy { "ok" } else { "warn" },
                format!(
                    "schema {} · {} codex provider(s) · {} with common config · {} codex MCP(s)",
                    summary.schema_version,
                    summary.codex_providers,
                    summary.providers_with_common_config,
                    summary.mcp_codex_enabled
                ),
            ));
        }
        Ok(None) => checks.push(check(
            "cc_switch",
            true,
            "not installed (optional)".to_string(),
        )),
        Err(error) => checks.push(check("cc_switch", false, error.to_string())),
    }
    match autostart::status(store) {
        Ok(status) if !status.supported => checks.push(check_status(
            "autostart",
            "warn",
            "not supported on this platform (use a systemd user unit on Linux)".to_string(),
        )),
        Ok(status) => {
            let installed = status.watch_task_installed
                || status.check_task_installed
                || status.startup_script_installed;
            checks.push(check_status(
                "autostart",
                if installed { "ok" } else { "warn" },
                format!(
                    "watch task: {}, periodic check: {}, startup script: {}",
                    installed_label(status.watch_task_installed),
                    installed_label(status.check_task_installed),
                    installed_label(status.startup_script_installed)
                ),
            ));
        }
        Err(error) => checks.push(check_status("autostart", "warn", error.to_string())),
    }
    let failed = checks.iter().any(|entry| entry.status == "fail");
    let report = DoctorReport {
        version: status.version.clone(),
        codex_home: status.codex_home.display().to_string(),
        store_dir: status.store_dir.display().to_string(),
        checks,
    };
    if json {
        print_json(&report)?;
    } else {
        println!("KeepMyConfig v{}", report.version);
        println!("Codex home: {}", report.codex_home);
        println!("Store     : {}", report.store_dir);
        for entry in &report.checks {
            println!(
                "  [{}] {}: {}",
                match entry.status.as_str() {
                    "ok" => "ok",
                    "warn" => "warn",
                    _ => "fail",
                },
                entry.name,
                entry.detail
            );
        }
    }
    if failed {
        Ok(ExitCode::from(1))
    // GitHub@OxygenAI Lab | OxygenAILab@S   tarsa ilsClover
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn check(name: &str, ok: bool, detail: String) -> DoctorCheck {
    check_status(name, if ok { "ok" } else { "fail" }, detail)
}

fn installed_label(value: bool) -> &'static str {
    if value {
        "installed"
    } else {
        "not installed"
    }
}

fn check_status(name: &str, status: &str, detail: String) -> DoctorCheck {
    DoctorCheck {
        name: name.to_string(),
        status: status.to_string(),
        detail,
    }
}

fn print_value<T: Serialize>(json: bool, value: &T, human: String) -> Result<()> {
    if json {
        print_json(value)
    } else {
        print!("{human}");
        Ok(())
    }
}

fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn parse_duration(text: &str) -> Result<Duration> {
    // GitHub@Oxyge   nAILab |  OxygenAILab@St   ars ailsClover
    let text = text.trim();
    let error = || anyhow::anyhow!("invalid duration '{text}': use values like 800ms, 30s, or 2m");
    if let Some(value) = text.strip_suffix("ms") {
        let millis: u64 = value.trim().parse().map_err(|_| error())?;
        return Ok(Duration::from_millis(millis));
    }
    if let Some(value) = text.strip_suffix('s') {
        let seconds: u64 = value.trim().parse().map_err(|_| error())?;
        return Ok(Duration::from_secs(seconds));
    }
    if let Some(value) = text.strip_suffix('m') {
        let minutes: u64 = value.trim().parse().map_err(|_| error())?;
        return Ok(Duration::from_secs(minutes * 60));
    }
    let seconds: u64 = text.parse().map_err(|_| error())?;
    Ok(Duration::from_secs(seconds))
}
