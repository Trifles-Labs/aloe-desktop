//! What the Aloe Code workspace shows about a project folder, read straight from disk.
//!
//! The branch comes from `.git/HEAD` rather than from running `git`: a repository's own config can
//! make `git status` execute arbitrary commands (`core.fsmonitor`), and showing a branch name is
//! not worth running code from a folder the user only granted for reading and editing.

use std::{fs, path::Path};

/// The checked-out branch, the short commit for a detached HEAD, or `None` outside a repository.
pub fn git_branch(root: &Path) -> Option<String> {
    let dot_git = root.join(".git");
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        // Worktrees and submodules: `.git` is a file pointing at the real directory.
        let pointer = fs::read_to_string(&dot_git).ok()?;
        let target = pointer.trim().strip_prefix("gitdir:")?.trim();
        let target = Path::new(target);
        if target.is_absolute() { target.to_path_buf() } else { root.join(target) }
    };
    let head = fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    match head.strip_prefix("ref:") {
        Some(reference) => {
            let reference = reference.trim();
            Some(reference.strip_prefix("refs/heads/").unwrap_or(reference).to_string())
        }
        None if head.len() >= 7 && head.chars().all(|c| c.is_ascii_hexdigit()) => Some(head[..7].to_string()),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("aloe-project-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_branch_from_head() {
        let dir = temp_dir("branch");
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::write(dir.join(".git/HEAD"), "ref: refs/heads/feature/code-mode\n").unwrap();
        assert_eq!(git_branch(&dir).as_deref(), Some("feature/code-mode"));
    }

    #[test]
    fn shortens_detached_head() {
        let dir = temp_dir("detached");
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::write(dir.join(".git/HEAD"), "0123456789abcdef0123456789abcdef01234567\n").unwrap();
        assert_eq!(git_branch(&dir).as_deref(), Some("0123456"));
    }

    #[test]
    fn follows_worktree_pointer() {
        let dir = temp_dir("worktree");
        let real = dir.join("real-git");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(dir.join(".git"), "gitdir: real-git\n").unwrap();
        assert_eq!(git_branch(&dir).as_deref(), Some("main"));
    }

    #[test]
    fn none_outside_a_repository() {
        assert_eq!(git_branch(&temp_dir("plain")), None);
    }
}
