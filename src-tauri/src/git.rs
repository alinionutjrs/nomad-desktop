use crate::models::Snapshot;
use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .context("Git nu este disponibil în PATH")?;
    if !output.status.success() {
        bail!("Git: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn ensure_repository(root: &Path) -> Result<()> {
    let owns_repository = git(root, &["rev-parse", "--show-toplevel"])
        .ok()
        .and_then(|value| std::fs::canonicalize(value).ok())
        .and_then(|repo_root| {
            std::fs::canonicalize(root)
                .ok()
                .map(|workspace_root| repo_root == workspace_root)
        })
        .unwrap_or(false);
    if !owns_repository {
        git(root, &["init", "--initial-branch=main"])?;
    }
    Ok(())
}

pub fn create_snapshot(root: &Path, message: &str) -> Result<String> {
    ensure_repository(root)?;
    git(root, &["add", "-A"])?;
    let status = git(root, &["status", "--porcelain"])?;
    let has_head = git(root, &["rev-parse", "--verify", "HEAD"]).is_ok();
    if status.is_empty() && has_head {
        return git(root, &["rev-parse", "HEAD"]);
    }
    let mut args = vec![
        "-c",
        "user.name=Nomad",
        "-c",
        "user.email=nomad@local",
        "commit",
        "--no-gpg-sign",
        "-m",
        message,
    ];
    if status.is_empty() {
        args.insert(5, "--allow-empty");
    }
    git(root, &args)?;
    git(root, &["rev-parse", "HEAD"])
}

pub fn list_snapshots(root: &Path) -> Result<Vec<Snapshot>> {
    if git(root, &["rev-parse", "--verify", "HEAD"]).is_err() {
        return Ok(Vec::new());
    }
    let output = git(root, &["log", "--pretty=format:%H%x1f%s%x1f%cI", "-50"])?;
    Ok(output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\u{1f}');
            Some(Snapshot {
                hash: parts.next()?.into(),
                message: parts.next()?.into(),
                created_at: parts.next()?.into(),
            })
        })
        .collect())
}

pub fn rollback(root: &Path, hash: &str) -> Result<String> {
    if hash.len() < 7 || hash.len() > 64 || !hash.chars().all(|value| value.is_ascii_hexdigit()) {
        bail!("Hash Git invalid");
    }
    git(root, &["cat-file", "-e", &format!("{hash}^{{commit}}")])?;
    let _safety = create_snapshot(root, "Nomad: snapshot înainte de rollback")?;
    git(
        root,
        &[
            "restore",
            "--source",
            hash,
            "--staged",
            "--worktree",
            "--",
            ".",
        ],
    )?;
    create_snapshot(
        root,
        &format!("Nomad: rollback la {}", &hash[..hash.len().min(9)]),
    )
}

pub fn diff(root: &Path) -> Result<String> {
    ensure_repository(root)?;
    git(root, &["diff", "--", "."])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_and_rollback_restore_files() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("hello.txt"), "one").unwrap();
        let first = create_snapshot(directory.path(), "first").unwrap();
        std::fs::write(directory.path().join("hello.txt"), "two").unwrap();
        create_snapshot(directory.path(), "second").unwrap();
        rollback(directory.path(), &first).unwrap();
        assert_eq!(
            std::fs::read_to_string(directory.path().join("hello.txt")).unwrap(),
            "one"
        );
    }
}
