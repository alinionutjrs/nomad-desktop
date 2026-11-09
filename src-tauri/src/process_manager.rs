use crate::models::ProcessStatus;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use std::collections::{HashMap, VecDeque};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

const MAX_LOG_LINES: usize = 1_000;
const ALLOWED_PROGRAMS: &[&str] = &[
    "npm",
    "npm.cmd",
    "npx",
    "npx.cmd",
    "pnpm",
    "pnpm.cmd",
    "yarn",
    "yarn.cmd",
    "node",
    "node.exe",
    "cargo",
    "cargo.exe",
    "git",
    "git.exe",
];

struct ManagedProcess {
    child: Child,
    pid: u32,
    started_at: String,
    logs: Arc<Mutex<VecDeque<String>>>,
}

#[derive(Default)]
pub struct ProcessManager {
    children: tokio::sync::Mutex<HashMap<i64, ManagedProcess>>,
}

pub fn validate_command(program: &str, args: &[String]) -> Result<()> {
    if !ALLOWED_PROGRAMS
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(program))
    {
        bail!("Programul '{program}' nu este în lista permisă: npm, npx, pnpm, yarn, node, cargo, git");
    }
    const SHELL_META: &[char] = &['&', '|', '<', '>', '^', '%', '\n', '\r', ';'];
    if args
        .iter()
        .any(|arg| arg.chars().any(|ch| SHELL_META.contains(&ch)))
    {
        bail!("Argumentele conțin caractere shell nepermise");
    }
    if args.len() > 100 || args.iter().any(|arg| arg.len() > 4_000) {
        bail!("Comanda este prea lungă");
    }
    Ok(())
}

pub fn command_executable(program: &str) -> &str {
    #[cfg(windows)]
    {
        match program.to_ascii_lowercase().as_str() {
            "npm" => "npm.cmd",
            "npx" => "npx.cmd",
            "pnpm" => "pnpm.cmd",
            "yarn" => "yarn.cmd",
            _ => program,
        }
    }
    #[cfg(not(windows))]
    {
        program
    }
}

fn push_log(logs: &Arc<Mutex<VecDeque<String>>>, prefix: &str, line: String) {
    if let Ok(mut values) = logs.lock() {
        if values.len() >= MAX_LOG_LINES {
            values.pop_front();
        }
        values.push_back(format!("{prefix}{line}"));
    }
}

impl ProcessManager {
    pub async fn start(
        &self,
        project_id: i64,
        root: &std::path::Path,
        program: &str,
        args: &[String],
    ) -> Result<ProcessStatus> {
        validate_command(program, args)?;
        let mut children = self.children.lock().await;
        if let Some(existing) = children.get_mut(&project_id) {
            if existing.child.try_wait()?.is_none() {
                bail!("Un dev-server rulează deja pentru acest proiect");
            }
        }
        children.remove(&project_id);

        let mut command = Command::new(command_executable(program));
        command
            .args(args)
            .current_dir(root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = command
            .spawn()
            .with_context(|| format!("Nu pot porni programul {program}"))?;
        let pid = child
            .id()
            .ok_or_else(|| anyhow::anyhow!("Procesul nu a returnat PID"))?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let logs = Arc::new(Mutex::new(VecDeque::new()));
        push_log(&logs, "", format!("$ {program} {}", args.join(" ")));

        if let Some(stdout) = stdout {
            let target = logs.clone();
            tauri::async_runtime::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    push_log(&target, "", line);
                }
            });
        }
        if let Some(stderr) = stderr {
            let target = logs.clone();
            tauri::async_runtime::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    push_log(&target, "! ", line);
                }
            });
        }
        let started_at = Utc::now().to_rfc3339();
        children.insert(
            project_id,
            ManagedProcess {
                child,
                pid,
                started_at: started_at.clone(),
                logs: logs.clone(),
            },
        );
        Ok(ProcessStatus {
            running: true,
            pid: Some(pid),
            started_at: Some(started_at),
            logs: Vec::new(),
        })
    }

    pub async fn stop(&self, project_id: i64) -> Result<()> {
        let mut children = self.children.lock().await;
        if let Some(process) = children.get_mut(&project_id) {
            if process.child.try_wait()?.is_none() {
                process
                    .child
                    .kill()
                    .await
                    .context("Nu pot opri dev-server-ul")?;
                let _ = process.child.wait().await;
                push_log(&process.logs, "", "[proces oprit]".into());
            }
        }
        Ok(())
    }

    pub async fn status(&self, project_id: i64) -> Result<ProcessStatus> {
        let mut children = self.children.lock().await;
        let Some(process) = children.get_mut(&project_id) else {
            return Ok(ProcessStatus {
                running: false,
                pid: None,
                started_at: None,
                logs: Vec::new(),
            });
        };
        let running = process.child.try_wait()?.is_none();
        let logs = process
            .logs
            .lock()
            .map(|value| value.iter().cloned().collect())
            .unwrap_or_default();
        Ok(ProcessStatus {
            running,
            pid: Some(process.pid),
            started_at: Some(process.started_at.clone()),
            logs,
        })
    }
}
