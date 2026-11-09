use crate::models::{CommandOutput, FileEntry, SearchMatch};
use crate::process_manager::{command_executable, validate_command};
use crate::security::Workspace;
use anyhow::{bail, Context, Result};
use std::fs;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use walkdir::{DirEntry, WalkDir};

pub const TOOL_NAMES: &[&str] = &[
    "read_file",
    "list_files",
    "search_files",
    "write_file",
    "apply_patch",
    "run_command",
    "git_diff",
];

const MAX_TEXT_FILE: u64 = 2 * 1024 * 1024;
const MAX_ENTRIES: usize = 2_500;

fn ignored(entry: &DirEntry) -> bool {
    entry.depth() > 0
        && entry.file_type().is_dir()
        && matches!(
            entry.file_name().to_string_lossy().as_ref(),
            ".git" | "node_modules" | "target" | "dist" | ".next" | ".turbo"
        )
}

fn portable(relative: &std::path::Path) -> String {
    relative.to_string_lossy().replace('\\', "/")
}

pub fn is_sensitive_path(path: &str) -> bool {
    if path.trim().is_empty() {
        return true;
    }
    if path
        .split(['/', '\\'])
        .map(str::to_ascii_lowercase)
        .any(|part| {
            matches!(
                part.as_str(),
                ".git" | ".ssh" | ".codex" | ".claude" | ".agents"
            )
        })
    {
        return true;
    }
    let name = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    name == ".env"
        || name.starts_with(".env.")
        || matches!(
            name.as_str(),
            ".npmrc"
                | ".pypirc"
                | ".netrc"
                | "credentials"
                | "credentials.json"
                | "secrets.json"
                | "auth.json"
                | "id_rsa"
                | "id_ed25519"
        )
        || name.contains("secret")
        || name == "token"
        || name.starts_with("token.")
        || name.starts_with("tokens.")
        || name.contains("_token.")
        || [".pem", ".key", ".p12", ".pfx", ".keystore"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

pub fn list_files(workspace: &Workspace) -> Result<Vec<FileEntry>> {
    let mut entries = Vec::new();
    for item in WalkDir::new(workspace.root())
        .follow_links(false)
        .max_depth(12)
        .into_iter()
        .filter_entry(|entry| !ignored(entry))
    {
        let item = item?;
        if item.depth() == 0 || item.file_type().is_symlink() {
            continue;
        }
        let relative = item.path().strip_prefix(workspace.root())?;
        entries.push(FileEntry {
            path: portable(relative),
            is_dir: item.file_type().is_dir(),
            size: item.metadata().map(|value| value.len()).unwrap_or(0),
            depth: relative.components().count().saturating_sub(1),
        });
        if entries.len() >= MAX_ENTRIES {
            break;
        }
    }
    entries.sort_by(|a, b| {
        let a_parent = a.path.rsplit_once('/').map(|value| value.0).unwrap_or("");
        let b_parent = b.path.rsplit_once('/').map(|value| value.0).unwrap_or("");
        a_parent
            .cmp(b_parent)
            .then_with(|| b.is_dir.cmp(&a.is_dir))
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(entries)
}

pub fn read_file(workspace: &Workspace, path: &str) -> Result<String> {
    let resolved = workspace.resolve_existing(path)?;
    let metadata = fs::metadata(&resolved)?;
    if !metadata.is_file() {
        bail!("Calea nu este un fișier");
    }
    if metadata.len() > MAX_TEXT_FILE {
        bail!("Fișierul depășește limita de 2 MB");
    }
    fs::read_to_string(&resolved).context("Fișierul nu este text UTF-8")
}

pub fn search_files(workspace: &Workspace, query: &str) -> Result<Vec<SearchMatch>> {
    if query.trim().is_empty() {
        bail!("Termenul de căutare este gol");
    }
    let needle = query.to_lowercase();
    let mut matches = Vec::new();
    for entry in list_files(workspace)?
        .into_iter()
        .filter(|entry| !entry.is_dir && entry.size <= MAX_TEXT_FILE)
    {
        let Ok(content) = read_file(workspace, &entry.path) else {
            continue;
        };
        for (index, line) in content.lines().enumerate() {
            if line.to_lowercase().contains(&needle) {
                matches.push(SearchMatch {
                    path: entry.path.clone(),
                    line: index + 1,
                    text: line.chars().take(300).collect(),
                });
                if matches.len() >= 200 {
                    return Ok(matches);
                }
            }
        }
    }
    Ok(matches)
}

pub fn write_file(workspace: &Workspace, path: &str, content: &str) -> Result<()> {
    if content.len() > MAX_TEXT_FILE as usize {
        bail!("Conținutul depășește limita de 2 MB");
    }
    let resolved = workspace.resolve_for_write(path)?;
    let temp = resolved.with_extension(format!("nomad-{}.tmp", uuid::Uuid::new_v4()));
    fs::write(&temp, content).context("Nu pot scrie fișierul temporar")?;
    if let Err(error) = fs::rename(&temp, &resolved) {
        let _ = fs::remove_file(&temp);
        return Err(error).context("Nu pot înlocui fișierul");
    }
    Ok(())
}

pub fn apply_patch(
    workspace: &Workspace,
    path: &str,
    expected: &str,
    replacement: &str,
) -> Result<()> {
    if expected.is_empty() {
        bail!("Fragmentul căutat nu poate fi gol");
    }
    let content = read_file(workspace, path)?;
    let occurrences = content.match_indices(expected).count();
    if occurrences != 1 {
        bail!("Patch-ul cere exact o potrivire, dar au fost găsite {occurrences}");
    }
    write_file(workspace, path, &content.replacen(expected, replacement, 1))
}

pub async fn run_command(
    workspace: &Workspace,
    program: &str,
    args: &[String],
    timeout_seconds: u64,
) -> Result<CommandOutput> {
    validate_command(program, args)?;
    let executable = command_executable(program);
    let mut command;
    #[cfg(windows)]
    {
        if executable.to_ascii_lowercase().ends_with(".cmd")
            || executable.to_ascii_lowercase().ends_with(".bat")
        {
            let target = executable.to_string();
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
        .current_dir(workspace.root())
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(
        Duration::from_secs(timeout_seconds.clamp(1, 3600)),
        command.output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("Comanda a depășit timeout-ul"))?
    .with_context(|| format!("Nu pot rula programul {program}"))?;
    Ok(CommandOutput {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout)
            .chars()
            .take(200_000)
            .collect(),
        stderr: String::from_utf8_lossy(&output.stderr)
            .chars()
            .take(200_000)
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_patch_refuses_ambiguous_matches() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("a.txt"), "x x").unwrap();
        let workspace = Workspace::new(directory.path()).unwrap();
        assert!(apply_patch(&workspace, "a.txt", "x", "y").is_err());
    }

    #[test]
    fn classifies_common_credential_files_as_sensitive() {
        assert!(is_sensitive_path(".env.local"));
        assert!(is_sensitive_path(".ssh/id_ed25519"));
        assert!(is_sensitive_path("config/.claude/settings.json"));
        assert!(!is_sensitive_path("src/tokenizer.rs"));
        assert!(!is_sensitive_path("src/main.rs"));
    }

    #[test]
    fn atomic_write_creates_and_replaces_files() {
        let directory = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(directory.path()).unwrap();
        write_file(&workspace, "one.txt", "first").unwrap();
        write_file(&workspace, "one.txt", "second").unwrap();
        assert_eq!(read_file(&workspace, "one.txt").unwrap(), "second");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
