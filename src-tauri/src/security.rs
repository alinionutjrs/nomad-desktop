use anyhow::{bail, Context, Result};
use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        let root = fs::canonicalize(root.as_ref())
            .with_context(|| format!("Workspace-ul {} nu există", root.as_ref().display()))?;
        if !root.is_dir() {
            bail!("Workspace-ul trebuie să fie un director");
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn components(relative: &str) -> Result<Vec<&OsStr>> {
        if relative.trim().is_empty() {
            bail!("Calea relativă este goală");
        }
        let path = Path::new(relative);
        if path.is_absolute() {
            bail!("Căile absolute nu sunt permise");
        }
        let mut values = Vec::new();
        for component in path.components() {
            match component {
                Component::Normal(value) => {
                    if value.to_string_lossy().contains(':') {
                        bail!("Caracterul ':' nu este permis în căi");
                    }
                    values.push(value);
                }
                _ => bail!("Calea conține o componentă nepermisă"),
            }
        }
        if values.is_empty() {
            bail!("Calea relativă este goală");
        }
        Ok(values)
    }

    fn joined(&self, relative: &str) -> Result<PathBuf> {
        let mut result = self.root.clone();
        for component in Self::components(relative)? {
            result.push(component);
        }
        Ok(result)
    }

    fn reject_symlink_chain(&self, path: &Path) -> Result<()> {
        let relative = path
            .strip_prefix(&self.root)
            .context("Calea iese din workspace")?;
        let mut current = self.root.clone();
        for component in relative.components() {
            current.push(component.as_os_str());
            if current.exists() && fs::symlink_metadata(&current)?.file_type().is_symlink() {
                bail!("Symlink-urile nu pot fi traversate: {}", current.display());
            }
        }
        Ok(())
    }

    pub fn resolve_existing(&self, relative: &str) -> Result<PathBuf> {
        let joined = self.joined(relative)?;
        self.reject_symlink_chain(&joined)?;
        let resolved = fs::canonicalize(&joined)
            .with_context(|| format!("Fișierul {} nu există", relative))?;
        if !resolved.starts_with(&self.root) {
            bail!("Calea iese din workspace");
        }
        Ok(resolved)
    }

    pub fn resolve_for_write(&self, relative: &str) -> Result<PathBuf> {
        let joined = self.joined(relative)?;
        let parent = joined.parent().context("Calea nu are director părinte")?;
        self.reject_symlink_chain(parent)?;
        let resolved_parent = fs::canonicalize(parent)
            .with_context(|| format!("Directorul părinte pentru {} nu există", relative))?;
        if !resolved_parent.starts_with(&self.root) {
            bail!("Calea iese din workspace");
        }
        if joined.exists() && fs::symlink_metadata(&joined)?.file_type().is_symlink() {
            bail!("Nu se poate scrie printr-un symlink");
        }
        Ok(joined)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_parent_traversal_and_absolute_paths() {
        let directory = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(directory.path()).unwrap();
        assert!(workspace.resolve_for_write("../secret.txt").is_err());
        assert!(workspace.resolve_for_write("C:\\secret.txt").is_err());
    }

    #[test]
    fn accepts_a_normal_child_path() {
        let directory = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(directory.path()).unwrap();
        assert_eq!(
            workspace.resolve_for_write("hello.txt").unwrap(),
            fs::canonicalize(directory.path())
                .unwrap()
                .join("hello.txt")
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), directory.path().join("link")).unwrap();
        let workspace = Workspace::new(directory.path()).unwrap();
        assert!(workspace.resolve_for_write("link/file.txt").is_err());
    }
}
