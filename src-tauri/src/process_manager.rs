use crate::models::ProcessStatus;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
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
    if args.len() > 100 || args.iter().any(|arg| arg.len() > 4_000) {
        bail!("Comanda este prea lungă");
    }
    if args.iter().any(|arg| arg.contains('\0')) {
        bail!("Argumentele conțin caractere nepermise");
    }
    #[cfg(windows)]
    if command_executable(program)
        .to_ascii_lowercase()
        .ends_with(".cmd")
        && args.iter().any(|arg| {
            arg.chars()
                .any(|ch| matches!(ch, '"' | '%' | '!' | '^' | '\r' | '\n'))
        })
    {
        bail!("Argumentele wrapper-ului Windows conțin caractere nepermise");
    }
    Ok(())
}

#[cfg(windows)]
fn resolve_batch_executable(executable: &str) -> PathBuf {
    let candidate = PathBuf::from(executable);
    if candidate.components().count() > 1 {
        return candidate;
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .map(|directory| directory.join(executable))
        .find(|path| path.is_file())
        .unwrap_or(candidate)
}

#[cfg(windows)]
fn resolve_node_cli(program: &str) -> Option<(PathBuf, PathBuf)> {
    let cli_name = match program.to_ascii_lowercase().as_str() {
        "npm" | "npm.cmd" => "npm-cli.js",
        "npx" | "npx.cmd" => "npx-cli.js",
        _ => return None,
    };
    let wrapper = if cli_name == "npm-cli.js" {
        "npm.cmd"
    } else {
        "npx.cmd"
    };
    let directory = resolve_batch_executable(wrapper).parent()?.to_path_buf();
    let node = directory.join("node.exe");
    let cli = directory
        .join("node_modules")
        .join("npm")
        .join("bin")
        .join(cli_name);
    (node.is_file() && cli.is_file()).then_some((node, cli))
}

pub fn split_command_line(value: &str) -> Result<Vec<String>> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' && quote == Some('"') {
            escaped = true;
            continue;
        }
        if let Some(open) = quote {
            if ch == open {
                quote = None;
            } else {
                current.push(ch);
            }
        } else if ch == '"' || ch == '\'' {
            quote = Some(ch);
        } else if ch.is_whitespace() {
            if !current.is_empty() {
                args.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }
    if quote.is_some() {
        bail!("Ghilimele neînchise în comandă");
    }
    if escaped {
        current.push('\\');
    }
    if !current.is_empty() {
        args.push(current);
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_normal_quoted_paths_without_shell_injection() {
        assert!(validate_command("npm", &["run".into(), "dev server".into()]).is_ok());
        assert!(validate_command(
            "npm",
            &["run".into(), "dev".into(), "&&".into(), "whoami".into()]
        )
        .is_ok());
        assert!(validate_command("powershell.exe", &["-Command".into(), "whoami".into()]).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn rejects_cmd_expansion_and_quote_characters_for_batch_wrappers() {
        assert!(validate_command("npm", &["run".into(), "dev".into()]).is_ok());
        assert!(validate_command("npm", &["run".into(), "%PATH%".into()]).is_err());
        assert!(validate_command("npm", &["run".into(), "a\" & whoami".into()]).is_err());
    }

    #[test]
    fn command_parser_keeps_quoted_paths_as_one_argument() {
        assert_eq!(
            split_command_line("npm run dev -- --config \"C:/Users/me/app config.json\"").unwrap(),
            vec![
                "npm",
                "run",
                "dev",
                "--",
                "--config",
                "C:/Users/me/app config.json"
            ]
        );
        assert!(split_command_line("npm run \"dev").is_err());
    }
}

pub fn command_executable(program: &str) -> &str {
    #[cfg(windows)]
    {
        match program.to_ascii_lowercase().as_str() {
            "npm" => "npm.cmd",
            "npx" => "npx.cmd",
            "pnpm" => "pnpm.cmd",
            "yarn" => "yarn.cmd",
            "node" => "node.exe",
            "cargo" => "cargo.exe",
            "git" => "git.exe",
            _ => program,
        }
    }
    #[cfg(not(windows))]
    {
        program
    }
}

#[cfg(unix)]
fn terminate_process_tree(pid: u32) -> Result<()> {
    let status = std::process::Command::new("pkill")
        .args(["-TERM", "-P", &pid.to_string()])
        .status();
    if let Err(error) = status {
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(error.into());
        }
    }
    let output = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .output();
    match output {
        Ok(result) if result.status.success() => Ok(()),
        Ok(result) => bail!(
            "Nu pot opri procesul: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
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

        let executable = command_executable(program);
        let mut command;
        #[cfg(windows)]
        {
            if let Some((node, cli)) = resolve_node_cli(program) {
                command = Command::new(node);
                command.arg(cli).args(args).creation_flags(0x08000000);
            } else if executable.to_ascii_lowercase().ends_with(".cmd")
                || executable.to_ascii_lowercase().ends_with(".bat")
            {
                let target = resolve_batch_executable(executable)
                    .to_string_lossy()
                    .into_owned();
                let quoted = format!("\"{}\"", target.replace('"', "\\\""));
                let suffix = std::iter::once(quoted)
                    .chain(
                        args.iter()
                            .map(|value| format!("\"{}\"", value.replace('"', "\\\""))),
                    )
                    .collect::<Vec<_>>()
                    .join(" ");
                command = Command::new("cmd.exe");
                command
                    .args(["/d", "/s", "/c"])
                    .raw_arg(format!("\"{suffix}\""));
                command.creation_flags(0x00000200 | 0x08000000);
            } else {
                command = Command::new(executable);
                command.args(args).creation_flags(0x08000000);
            }
        }
        #[cfg(not(windows))]
        {
            command = Command::new(executable);
            command.args(args);
        }
        command
            .current_dir(root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
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
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    use std::process::Command as StdCommand;
                    let status = StdCommand::new("taskkill")
                        .args(["/PID", &process.pid.to_string(), "/T", "/F"])
                        .creation_flags(0x08000000)
                        .status()
                        .context("Nu pot opri arborele dev-server-ului")?;
                    if !status.success() {
                        process
                            .child
                            .kill()
                            .await
                            .context("Nu pot opri dev-server-ul")?;
                    }
                }
                #[cfg(not(windows))]
                terminate_process_tree(process.pid)?;
                #[cfg(not(windows))]
                {
                    let _ = process.child.try_wait();
                }
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
