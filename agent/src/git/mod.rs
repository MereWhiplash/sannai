pub mod observer;
pub mod tool_detect;

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

#[derive(Debug, Clone, PartialEq)]
pub enum HeadChangeCause {
    Commit,
    Amend,
    Rebase,
    Checkout,
    Reset,
    Merge,
    CherryPick,
    Unknown,
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

pub fn infer_head_change_cause(
    repo_path: &Path,
    old_sha: &str,
    new_sha: &str,
) -> anyhow::Result<HeadChangeCause> {
    if old_sha == new_sha {
        return Ok(HeadChangeCause::Unknown);
    }

    let repo = Repository::open(repo_path)?;
    let old_oid = git2::Oid::from_str(old_sha)?;
    let new_oid = git2::Oid::from_str(new_sha)?;
    let new_commit = repo.find_commit(new_oid)?;

    // Merge: multiple parents
    if new_commit.parent_count() > 1 {
        return Ok(HeadChangeCause::Merge);
    }

    // Check if old_sha is direct parent of new_sha (normal commit)
    if new_commit.parent_count() == 1 {
        let parent = new_commit.parent(0)?;
        if parent.id() == old_oid {
            return Ok(HeadChangeCause::Commit);
        }

        // Same parent as old commit = amend
        if let Ok(old_commit) = repo.find_commit(old_oid) {
            if old_commit.parent_count() == 1 && old_commit.parent(0)?.id() == parent.id() {
                return Ok(HeadChangeCause::Amend);
            }
        }
    }

    // new_sha is ancestor of old_sha = reset (went backwards)
    if repo.graph_descendant_of(old_oid, new_oid)? {
        return Ok(HeadChangeCause::Reset);
    }

    // old_sha is ancestor of new_sha but not direct parent = rebase or fast-forward
    if repo.graph_descendant_of(new_oid, old_oid)? {
        return Ok(HeadChangeCause::Rebase);
    }

    // Diverged — likely checkout or cherry-pick
    Ok(HeadChangeCause::Unknown)
}

pub struct CommitDetails {
    pub sha: String,
    pub parent_shas: Vec<String>,
    pub message: String,
    pub author: String,
    pub files_changed: Vec<String>,
    pub insertions: u32,
    pub deletions: u32,
}

pub fn get_commit_details(repo_path: &Path, sha: &str) -> anyhow::Result<CommitDetails> {
    let repo = Repository::open(repo_path)?;
    let oid = git2::Oid::from_str(sha)?;
    let commit = repo.find_commit(oid)?;

    let message = commit.message().unwrap_or("").trim().to_string();
    let author = commit.author().name().unwrap_or("unknown").to_string();
    let parent_shas: Vec<String> = commit.parent_ids().map(|id| id.to_string()).collect();

    let tree = commit.tree()?;
    let parent_tree = if commit.parent_count() > 0 {
        Some(commit.parent(0)?.tree()?)
    } else {
        None
    };

    let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)?;
    let stats = diff.stats()?;

    let mut files_changed = Vec::new();
    diff.foreach(
        &mut |delta, _| {
            if let Some(path) = delta.new_file().path().and_then(|p| p.to_str()) {
                files_changed.push(path.to_string());
            }
            true
        },
        None,
        None,
        None,
    )?;

    Ok(CommitDetails {
        sha: sha.to_string(),
        parent_shas,
        message,
        author,
        files_changed,
        insertions: stats.insertions() as u32,
        deletions: stats.deletions() as u32,
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

    #[test]
    fn test_get_commit_details() {
        let dir = init_test_repo();

        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "add main"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let state = read_repo_state(dir.path()).unwrap();
        let details = get_commit_details(dir.path(), &state.head_sha).unwrap();

        assert_eq!(details.message, "add main");
        assert!(!details.parent_shas.is_empty());
        assert!(details.files_changed.contains(&"src/main.rs".to_string()));
        assert!(details.insertions > 0);
    }

    #[test]
    fn test_infer_cause_commit() {
        let dir = init_test_repo();
        let state_before = read_repo_state(dir.path()).unwrap();

        std::fs::write(dir.path().join("file.txt"), "content").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "second"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let state_after = read_repo_state(dir.path()).unwrap();
        let cause =
            infer_head_change_cause(dir.path(), &state_before.head_sha, &state_after.head_sha)
                .unwrap();
        assert_eq!(cause, HeadChangeCause::Commit);
    }

    #[test]
    fn test_infer_cause_amend() {
        let dir = init_test_repo();

        std::fs::write(dir.path().join("file.txt"), "v1").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "original"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let state_before = read_repo_state(dir.path()).unwrap();

        std::fs::write(dir.path().join("file.txt"), "v2").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "--amend", "-m", "amended"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let state_after = read_repo_state(dir.path()).unwrap();
        let cause =
            infer_head_change_cause(dir.path(), &state_before.head_sha, &state_after.head_sha)
                .unwrap();
        assert_eq!(cause, HeadChangeCause::Amend);
    }

    #[test]
    fn test_infer_cause_reset() {
        let dir = init_test_repo();
        let initial_sha = read_repo_state(dir.path()).unwrap().head_sha;

        std::fs::write(dir.path().join("file.txt"), "v1").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "c1"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let state_before = read_repo_state(dir.path()).unwrap();

        Command::new("git")
            .args(["reset", "--hard", &initial_sha])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let state_after = read_repo_state(dir.path()).unwrap();
        let cause =
            infer_head_change_cause(dir.path(), &state_before.head_sha, &state_after.head_sha)
                .unwrap();
        assert_eq!(cause, HeadChangeCause::Reset);
    }
}
