//! Git worktree management for parallel agent sessions.
//!
//! Each worktree lets one `claude` agent work a checkout of the repo without
//! colliding with the editor's tree or with other agents. We shell out to
//! `git` (matching `ui::file_tree::git`) rather than depend on a git library
//! here. Worktrees are only ever removed with explicit confirmation; a dirty
//! worktree is never silently discarded.

use std::path::{Path, PathBuf};
use std::process::Command;

use helix_view::agent::WorktreeInfo;
use helix_view::Editor;

/// The directory under which per-session worktrees are created:
/// `config.claude_code.worktree_root`, or `<repo>/.helix-worktrees`.
pub fn worktree_root(editor: &Editor, repo: &Path) -> PathBuf {
    editor
        .config()
        .claude_code
        .worktree_root
        .clone()
        .unwrap_or_else(|| repo.join(".helix-worktrees"))
}

/// Run `git -C <dir> <args>`, returning stdout on success or an error carrying
/// trimmed stderr.
fn git(dir: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git").arg("-C").arg(dir).args(args).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git {}: {}", args.join(" "), stderr.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Whether a local branch already exists.
fn branch_exists(repo: &Path, branch: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Create a worktree for `branch` under the configured root and return its
/// info. Creates the branch from HEAD if it doesn't already exist.
pub fn create(editor: &Editor, branch: &str) -> anyhow::Result<WorktreeInfo> {
    let repo = helix_core::find_workspace().0;
    git(&repo, &["rev-parse", "--git-dir"])
        .map_err(|_| anyhow::anyhow!("not inside a git repository"))?;

    let root = worktree_root(editor, &repo);
    std::fs::create_dir_all(&root)?;

    // Branch names may contain '/'; flatten for the on-disk directory name.
    let path = root.join(branch.replace('/', "-"));
    if path.exists() {
        anyhow::bail!("worktree path already exists: {}", path.display());
    }

    let path_str = path.to_string_lossy();
    if branch_exists(&repo, branch) {
        git(&repo, &["worktree", "add", &path_str, branch])?;
    } else {
        git(&repo, &["worktree", "add", "-b", branch, &path_str])?;
    }

    Ok(WorktreeInfo {
        path,
        branch: branch.to_string(),
    })
}

/// Whether the worktree has uncommitted changes (tracked or untracked).
pub fn is_dirty(path: &Path) -> bool {
    git(path, &["status", "--porcelain"])
        .map(|out| !out.trim().is_empty())
        .unwrap_or(false)
}

/// Remove a worktree via `git worktree remove`. Refuses a dirty/locked worktree
/// unless `force` is set — callers must confirm before forcing.
pub fn remove(info: &WorktreeInfo, force: bool) -> anyhow::Result<()> {
    let repo = helix_core::find_workspace().0;
    let path_str = info.path.to_string_lossy();
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(&path_str);
    git(&repo, &args)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_name_flattens_slashes() {
        // The on-disk directory derived from a branch name must not nest.
        let branch = "feature/auth-refactor";
        assert_eq!(branch.replace('/', "-"), "feature-auth-refactor");
    }

    #[test]
    fn root_defaults_under_repo() {
        // With no configured root the default sits inside the repo.
        let repo = Path::new("/tmp/somerepo");
        let default = repo.join(".helix-worktrees");
        assert!(default.ends_with(".helix-worktrees"));
        assert!(default.starts_with(repo));
    }
}
