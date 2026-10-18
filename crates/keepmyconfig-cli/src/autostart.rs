use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use keepmyconfig_core::store::Store;
use keepmyconfig_core::util;
use serde::Serialize;

const TASK_WATCH: &str = "KeepMyConfig-Watch";
// G i tHub@Oxyge  nAILa  b | OxygenAILab@Starsai lsC  lo  v er
const TASK_CHECK: &str = "KeepMyConfig-Check";
const WRAPPER_WATCH: &str = "watch-daemon.cmd";
const WRAPPER_CHECK: &str = "watch-once.cmd";
const STARTUP_SCRIPT: &str = "KeepMyConfig.cmd";
const BUS_WRAPPER_PREFIX: &str = "bus-watch-";
const TASK_BUS_PREFIX: &str = "KeepMyConfig-Bus-";

/// A channel watched on behalf of an actor that cannot run itself (Ops).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusWatch {
    pub channel: String,
    pub actor: String,
}

impl BusWatch {
    /// Wrap a label so only filesystem- and task-safe characters remain, while
    /// staying readable in the Startup folder and Task Scheduler.
    pub fn slug(&self) -> String {
        let raw = format!("{}-{}", self.channel, self.actor);
        let mut out = String::new();
        let mut last_dash = false;
        for ch in raw.chars() {
            if ch.is_ascii_alphanumeric() {
                out.push(ch);
                last_dash = false;
            } else if !last_dash {
                out.push('-');
                last_dash = true;
            }
        }
        out.trim_matches('-').to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMethod {
    Auto,
    Task,
    Startup,
}

#[derive(Debug, Clone, Serialize)]
pub struct AutostartStatus {
    pub supported: bool,
    pub executable: PathBuf,
    pub codex_home: PathBuf,
    pub store_dir: PathBuf,
    pub watch_task_installed: bool,
    pub check_task_installed: bool,
    pub watch_wrapper: PathBuf,
    pub check_wrapper: PathBuf,
    pub startup_script: Option<PathBuf>,
    pub startup_script_installed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AutostartReport {
    pub action: String,
    pub details: Vec<String>,
    pub status: AutostartStatus,
}

pub fn status(store: &Store) -> Result<AutostartStatus> {
    let executable = std::env::current_exe().context("cannot resolve the current executable")?;
    let wrapper_dir = executable
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let startup_script = startup_script_path();
    Ok(AutostartStatus {
        supported: cfg!(windows),
        executable,
        codex_home: store.paths().codex_home.clone(),
        store_dir: store.paths().store_dir.clone(),
        watch_task_installed: task_exists(TASK_WATCH),
        check_task_installed: task_exists(TASK_CHECK),
        watch_wrapper: wrapper_dir.join(WRAPPER_WATCH),
        check_wrapper: wrapper_dir.join(WRAPPER_CHECK),
        startup_script_installed: startup_script.as_deref().is_some_and(Path::is_file),
        startup_script,
    })
}

/// Install self-healing protection for the current user.
///
/// `Auto` prefers Task Scheduler (logon daemon + periodic check) and falls back
/// to a Startup-folder loop when the host denies task creation, which is the
/// common outcome for non-elevated Windows sessions.
// Git Hub @OxygenAILab | OxygenAI Lab@S   tarsa ilsClover
pub fn install(
    store: &Store,
    interval_minutes: u64,
    no_daemon: bool,
    method: InstallMethod,
    bus_watches: &[BusWatch],
    cowork: &Path,
) -> Result<AutostartReport> {
    if !cfg!(windows) {
        bail!(
            "autostart is currently implemented for Windows only; on Linux use a systemd user unit running `keepmyconfig watch`"
        );
    }
    if interval_minutes == 0 || interval_minutes > 1440 {
        bail!("--interval-minutes must be between 1 and 1440");
    }
    let current = status(store)?;
    let mut details = Vec::new();
    let mut tasks_installed = false;

    if method != InstallMethod::Startup {
        match create_tasks(&current, interval_minutes, no_daemon, bus_watches, cowork) {
            Ok(task_details) => {
                tasks_installed = true;
                details.extend(task_details);
            }
            Err(error) if method == InstallMethod::Auto => {
                details.push(format!(
                    "Task Scheduler unavailable ({error:#}); falling back to the Startup folder"
                ));
                let _ = remove_tasks();
            }
            Err(error) => return Err(error),
        }
    }

    if tasks_installed {
        if !no_daemon {
            let _ = run_schtasks(&["/Run", "/TN", TASK_WATCH]);
            details.push(format!("started {TASK_WATCH} now"));
        }
        let _ = run_schtasks(&["/Run", "/TN", TASK_CHECK]);
        details.push(format!("started {TASK_CHECK} now"));
    } else {
        let startup_script = current
            .startup_script
            .clone()
            .ok_or_else(|| anyhow::anyhow!("cannot resolve the Startup folder"))?;
        util::atomic_write(
            &startup_script,
            &startup_loop_script(
                &current.executable,
                &current.codex_home,
                &current.store_dir,
                interval_minutes,
                bus_watches,
                cowork,
            ),
        )
        .with_context(|| format!("cannot write {}", startup_script.display()))?;
        details.push(format!(
            "startup script: {} (runs `watch --once` every {interval_minutes} minute(s))",
            startup_script.display()
        ));
        start_detached(&current);
        details.push("started one watch helper now".to_string());
    }

    Ok(AutostartReport {
        action: "install".to_string(),
        details,
        status: status(store)?,
    })
}

pub fn uninstall(store: &Store) -> Result<AutostartReport> {
    if !cfg!(windows) {
        bail!("autostart is currently implemented for Windows only");
    }
    let current = status(store)?;
    let mut details = Vec::new();
    let mut tasks: Vec<String> = vec![TASK_WATCH.to_string(), TASK_CHECK.to_string()];
    // Discover any bus tasks so uninstall cleans up after a changed watch list.
    if let Ok(output) = Command::new("schtasks")
        .args(["/Query", "/FO", "LIST"])
        .output()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let trimmed = line.trim();
            if let Some(name) = trimmed.strip_prefix("TaskName:") {
                let name = name.trim();
                if name.contains(TASK_BUS_PREFIX) {
                    tasks.push(name.rsplit('\\').next().unwrap_or(name).to_string());
                }
            }
        }
    }
    for task in &tasks {
        if task_exists(task) {
            match run_schtasks(&["/Delete", "/F", "/TN", task]) {
                // Git   Hub@Oxygen AILab | OxygenAILab@Stars  ailsC lover
                Ok(()) => details.push(format!("removed task: {task}")),
                Err(error) => details.push(format!("could not remove task {task}: {error}")),
            }
        }
    }
    if let Some(startup_script) = &current.startup_script {
        if startup_script.is_file() {
            std::fs::remove_file(startup_script)
                .with_context(|| format!("cannot remove {}", startup_script.display()))?;
            details.push(format!(
                "removed startup script: {}",
                startup_script.display()
            ));
        }
    }
    let (launcher_ps1, launcher_vbs) = launcher_paths(&current);
    for artifact in [
        &current.watch_wrapper,
        &current.check_wrapper,
        &launcher_ps1,
        &launcher_vbs,
    ] {
        if artifact.is_file() {
            std::fs::remove_file(artifact)
                .with_context(|| format!("cannot remove {}", artifact.display()))?;
            details.push(format!("removed launcher: {}", artifact.display()));
        }
    }
    Ok(AutostartReport {
        action: "uninstall".to_string(),
        details,
        status: status(store)?,
    })
}

fn create_tasks(
    current: &AutostartStatus,
    interval_minutes: u64,
    no_daemon: bool,
    bus_watches: &[BusWatch],
    cowork: &Path,
) -> Result<Vec<String>> {
    let mut details = Vec::new();
    util::atomic_write(
        &current.watch_wrapper,
        &wrapper_script(
            &current.executable,
            &current.codex_home,
            &current.store_dir,
            false,
        ),
    )
    .with_context(|| format!("cannot write {}", current.watch_wrapper.display()))?;
    util::atomic_write(
        &current.check_wrapper,
        &wrapper_script(
            &current.executable,
            &current.codex_home,
            &current.store_dir,
            true,
        ),
    )
    .with_context(|| format!("cannot write {}", current.check_wrapper.display()))?;
    details.push(format!("wrappers: {}", current.watch_wrapper.display()));

    if !no_daemon {
        run_schtasks(&[
            "/Create",
            "/F",
            "/TN",
            TASK_WATCH,
            "/SC",
            "ONLOGON",
            "/TR",
            &format!("\"{}\"", current.watch_wrapper.display()),
        ])
        .context("cannot create the logon task")?;
        details.push(format!("task: {TASK_WATCH} (at logon, daemon)"));
    }
    run_schtasks(&[
        "/Create",
        "/F",
        "/TN",
        TASK_CHECK,
        "/SC",
        "MINUTE",
        "/MO",
        &interval_minutes.to_string(),
        "/TR",
        &format!("\"{}\"", current.check_wrapper.display()),
    ])
    .context("cannot create the periodic check task")?;
    details.push(format!(
        "task: {TASK_CHECK} (every {interval_minutes} minute(s), one-shot repair)"
    ));

    if !bus_watches.is_empty() {
        let label = bus_watches
            .iter()
            .map(BusWatch::slug)
            .collect::<Vec<_>>()
            .join("_");
        let mut bus_lines = String::from("@echo off\r\nsetlocal\r\n");
        for watch in bus_watches {
            bus_lines.push_str(&format!(
                "\"{}\" --cowork \"{}\" bus watch --once --quiet --channel \"{}\" --as \"{}\"\r\n",
                current.executable.display(),
                cowork.display(),
                watch.channel,
                watch.actor
            ));
        }
        let bus_wrapper = current
            .watch_wrapper
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(format!("{BUS_WRAPPER_PREFIX}{label}.cmd"));
        util::atomic_write(&bus_wrapper, &bus_lines)
            .with_context(|| format!("cannot write {}", bus_wrapper.display()))?;
        let task = format!("{TASK_BUS_PREFIX}{label}");
        run_schtasks(&[
            "/Create",
            "/F",
            "/TN",
            &task,
            "/SC",
            "MINUTE",
            "/MO",
            &interval_minutes.to_string(),
            "/TR",
            &format!("\"{}\"", bus_wrapper.display()),
        ])
        .context("cannot create the collaboration-bus task")?;
        details.push(format!(
            "task: {task} (every {interval_minutes} minute(s), notify-only bus watch)"
        ));
    }
    Ok(details)
}

fn remove_tasks() -> Result<()> {
    for task in [TASK_WATCH, TASK_CHECK] {
        if task_exists(task) {
            // GitHub@Oxyge nAILab | OxygenAILa  b@Sta   rsailsClover
            run_schtasks(&["/Delete", "/F", "/TN", task])?;
        }
    }
    Ok(())
}

fn wrapper_script(executable: &Path, codex_home: &Path, store_dir: &Path, once: bool) -> String {
    let mode = if once {
        "watch --once --quiet"
    } else {
        "watch --quiet"
    };
    format!(
        "@echo off\r\n\"{}\" --codex-home \"{}\" --store \"{}\" {mode}\r\n",
        executable.display(),
        codex_home.display(),
        store_dir.display()
    )
}

fn startup_loop_script(
    executable: &Path,
    codex_home: &Path,
    store_dir: &Path,
    interval_minutes: u64,
    bus_watches: &[BusWatch],
    cowork: &Path,
) -> String {
    // G  it Hub@Oxyge nAILab  | OxygenAILab@Star   sail  sCl  o   ve  r
    let seconds = interval_minutes * 60;
    let mut body = String::new();
    body.push_str(&format!(
        "\"{}\" --codex-home \"{}\" --store \"{}\" watch --once --quiet\r\n",
        executable.display(),
        codex_home.display(),
        store_dir.display()
    ));
    for watch in bus_watches {
        body.push_str(&format!(
            "\"{}\" --cowork \"{}\" bus watch --once --quiet --channel \"{}\" --as \"{}\"\r\n",
            executable.display(),
            cowork.display(),
            watch.channel,
            watch.actor
        ));
    }
    format!(
        "@echo off\r\nsetlocal\r\n:loop\r\n{body}timeout /t {seconds} /nobreak >nul\r\ngoto loop\r\n"
    )
}

fn launcher_paths(current: &AutostartStatus) -> (PathBuf, PathBuf) {
    let directory = current
        .watch_wrapper
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    (
        directory.join("launch-startup-loop.ps1"),
        directory.join("launch-startup-loop.vbs"),
    )
}

/// Start the Startup loop outside the caller's job object by asking the WMI
/// service to create the process. `cmd /C start` and `DETACHED_PROCESS` still
/// inherit the parent job on some hosts, which killed the helper as soon as
/// the installer session ended.
fn start_detached(current: &AutostartStatus) {
    let Some(startup_script) = current.startup_script.as_ref() else {
        return;
    };
    let (launcher_ps1, launcher_vbs) = launcher_paths(current);
    if util::atomic_write(&launcher_vbs, &hidden_launcher_vbs(startup_script)).is_err() {
        return;
    }
    if util::atomic_write(&launcher_ps1, &wmi_launcher_ps1(&launcher_vbs)).is_err() {
        return;
    }
    let mut command = Command::new("powershell");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&launcher_ps1)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let _ = command.spawn();
}

fn hidden_launcher_vbs(startup_script: &Path) -> String {
    format!(
        "Set sh = CreateObject(\"WScript.Shell\")\r\nq = Chr(34)\r\nsh.Run \"cmd.exe /c \" & q & q & \"{}\" & q & q, 0, False\r\n",
        startup_script.display()
    )
}

fn wmi_launcher_ps1(launcher_vbs: &Path) -> String {
    format!(
        "$cmd = 'wscript.exe \"\"{}\"\"'\r\nInvoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{{ CommandLine = $cmd }} | Out-Null\r\n",
        launcher_vbs.display()
    )
}

fn startup_script_path() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(
        PathBuf::from(appdata)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Startup")
            .join(STARTUP_SCRIPT),
    )
}

fn task_exists(name: &str) -> bool {
    if !cfg!(windows) {
        return false;
    }
    Command::new("schtasks")
        .args(["/Query", "/TN", name])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn run_schtasks(args: &[&str]) -> Result<()> {
    let output = Command::new("schtasks")
        .args(args)
        .output()
        .context("cannot run schtasks")?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        bail!(
            "schtasks {} failed: {}{}",
            args.join(" "),
            stdout.trim(),
            stderr.trim()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        hidden_launcher_vbs, startup_loop_script, wmi_launcher_ps1, wrapper_script, BusWatch,
    };
    use std::path::Path;

    #[test]
    fn wrapper_contains_resolved_paths_and_mode() {
        let script = wrapper_script(
            // GitHub@Oxyg enAILab | OxygenAILab@Starsai lsClov er
            Path::new(r"C:\Apps\KeepMyConfig\keepmyconfig.exe"),
            Path::new(r"C:\Users\demo\.codex"),
            Path::new(r"C:\Users\demo\.codex\.keepmyconfig"),
            true,
        );
        assert!(script.contains("--codex-home \"C:\\Users\\demo\\.codex\""));
        assert!(script.contains("watch --once --quiet"));
        assert!(script.contains("keepmyconfig.exe"));
    }

    #[test]
    fn startup_loop_runs_periodic_one_shot_repairs() {
        let script = startup_loop_script(
            Path::new(r"C:\Apps\KeepMyConfig\keepmyconfig.exe"),
            Path::new(r"C:\Users\demo\.codex"),
            Path::new(r"C:\Users\demo\.codex\.keepmyconfig"),
            5,
            &[],
            Path::new(r"C:\Users\demo\Documents\Cowork"),
        );
        assert!(script.contains(":loop"));
        assert!(script.contains("watch --once --quiet"));
        assert!(script.contains("timeout /t 300 /nobreak"));
        assert!(script.contains("goto loop"));
    }

    #[test]
    fn startup_loop_includes_bus_watches() {
        let watches = vec![BusWatch {
            channel: "env-ops".to_string(),
            actor: "Ops".to_string(),
        }];
        let script = startup_loop_script(
            Path::new(r"C:\Apps\KeepMyConfig\keepmyconfig.exe"),
            Path::new(r"C:\Users\demo\.codex"),
            Path::new(r"C:\Users\demo\.codex\.keepmyconfig"),
            5,
            &watches,
            Path::new(r"C:\Users\demo\Documents\Cowork"),
        );
        assert!(script.contains("bus watch --once --quiet --channel \"env-ops\" --as \"Ops\""));
        assert!(script.contains("--cowork \"C:\\Users\\demo\\Documents\\Cowork\""));
    }

    #[test]
    fn bus_watch_slug_is_task_safe() {
        let watch = BusWatch {
            channel: "DemoApp+UI".to_string(),
            actor: "Ops".to_string(),
        };
        assert_eq!(watch.slug(), "DemoApp-UI-Ops");
    }

    #[test]
    fn launcher_uses_wmi_and_a_hidden_window() {
        let vbs = hidden_launcher_vbs(Path::new(r"C:\Startup\KeepMyConfig.cmd"));
        assert!(vbs.contains("Chr(34)"));
        assert!(vbs.contains(", 0, False"));
        let ps1 = wmi_launcher_ps1(Path::new(r"C:\Apps\launch-startup-loop.vbs"));
        assert!(ps1.contains("Invoke-CimMethod"));
        assert!(ps1.contains("wscript.exe"));
        assert!(ps1.contains("launch-startup-loop.vbs"));
    }
}
