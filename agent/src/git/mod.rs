use git2::Repository;
use std::path::{Path, PathBuf};

pub struct RepoState {
    pub head_sha: String,
    pub branch: Option<String>,
    pub dirty_files: Vec<DirtyFile>,
}

pub struct DirtyFile {
    pub path: String,
    pub status: FileStatus,
}

pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
}

pub fn discover_repo(path: &Path) -> Option<PathBuf> {
    Repository::discover(path)
        .ok()
        .and_then(|repo| repo.workdir().map(|p| p.canonicalize().unwrap_or_else(|_| p.to_path_buf())))
}

pub fn read_repo_state(repo_path: &Path) -> anyhow::Result<RepoState> {
    let repo = Repository::open(repo_path)?;
    let head = repo.head()?;
    let head_sha = head.peel_to_commit()?.id().to_string();
    let branch = head.shorthand().map(|s| s.to_string());

    let mut dirty_files = Vec::new();
    let statuses = repo.statuses(None)?;
    for entry in statuses.iter() {
        if let Some(path) = entry.path() {
            let status = entry.status();
            let file_status = if status.intersects(git2::Status::WT_NEW | git2::Status::INDEX_NEW) {
                FileStatus::Added
            } else if status.intersects(git2::Status::WT_DELETED | git2::Status::INDEX_DELETED) {
                FileStatus::Deleted
            } else if status.intersects(git2::Status::WT_RENAMED | git2::Status::INDEX_RENAMED) {
                FileStatus::Renamed
            } else {
                FileStatus::Modified
            };
            dirty_files.push(DirtyFile {
                path: path.to_string(),
                status: file_status,
            });
        }
    }

    Ok(RepoState {
        head_sha,
        branch,
        dirty_files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn init_test_repo() -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        // Initial commit
        std::fs::write(dir.path().join("README.md"), "# test").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        dir
    }

    #[test]
    fn test_discover_repo() {
        let dir = init_test_repo();
        let subdir = dir.path().join("src");
        std::fs::create_dir(&subdir).unwrap();

        // Should find repo from subdirectory
        let repo_path = discover_repo(&subdir).unwrap();
        assert_eq!(repo_path, dir.path().canonicalize().unwrap());

        // Should fail for non-repo
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(discover_repo(tmp.path()).is_none());
    }

    #[test]
    fn test_read_repo_state() {
        let dir = init_test_repo();
        let state = read_repo_state(dir.path()).unwrap();

        assert!(!state.head_sha.is_empty());
        assert!(state.branch.is_some());
        assert!(state.dirty_files.is_empty());
    }

    #[test]
    fn test_read_repo_state_dirty() {
        let dir = init_test_repo();
        std::fs::write(dir.path().join("new.txt"), "dirty").unwrap();

        let state = read_repo_state(dir.path()).unwrap();
        assert_eq!(state.dirty_files.len(), 1);
        assert_eq!(state.dirty_files[0].path, "new.txt");
    }
}
