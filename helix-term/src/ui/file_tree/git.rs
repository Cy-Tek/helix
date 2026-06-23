use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitBadge {
    Modified,
    Added,
    Deleted,
    Untracked,
    Ignored,
}

pub type GitBadgeMap = BTreeMap<PathBuf, GitBadge>;

pub fn load_git_badges(root: &Path) -> GitBadgeMap {
    let Ok(output) = Command::new("git")
        .args(["-C"])
        .arg(root)
        .args(["status", "--porcelain=v1", "-z", "--ignored=matching"])
        .output()
    else {
        return GitBadgeMap::new();
    };

    if !output.status.success() {
        return GitBadgeMap::new();
    }

    parse_porcelain_status(root, &output.stdout)
}

pub fn parse_porcelain_status(root: &Path, output: &[u8]) -> GitBadgeMap {
    output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .filter_map(|record| parse_record(root, record))
        .collect()
}

fn parse_record(root: &Path, record: &[u8]) -> Option<(PathBuf, GitBadge)> {
    if record.len() < 4 {
        return None;
    }
    let x = record[0] as char;
    let y = record[1] as char;
    let path = std::str::from_utf8(&record[3..]).ok()?;
    let badge = match (x, y) {
        ('?', '?') => GitBadge::Untracked,
        ('!', '!') => GitBadge::Ignored,
        ('A', _) | (_, 'A') => GitBadge::Added,
        ('D', _) | (_, 'D') => GitBadge::Deleted,
        ('M', _) | (_, 'M') => GitBadge::Modified,
        _ => GitBadge::Modified,
    };
    Some((root.join(path), badge))
}
