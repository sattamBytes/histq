//! Recording commands: context capture (cwd, git) and record-start/record-end.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::db::{Db, NewEntry};
use crate::redact;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Git repo root and branch, discovered by walking up the directory tree.
/// Deliberately avoids spawning `git` — a subprocess costs more than the
/// entire rest of a keypress (~5-10ms vs ~µs for a few file reads).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GitContext {
    pub root: Option<String>,
    pub branch: Option<String>,
}

pub fn git_context(start: &Path) -> GitContext {
    let mut dir = start;
    loop {
        let dotgit = dir.join(".git");
        if dotgit.is_dir() {
            return GitContext {
                root: Some(dir.to_string_lossy().into_owned()),
                branch: read_head(&dotgit),
            };
        }
        if dotgit.is_file() {
            // Worktree or submodule: `.git` is a file containing `gitdir: <path>`.
            let gitdir = fs::read_to_string(&dotgit)
                .ok()
                .and_then(|s| {
                    s.trim()
                        .strip_prefix("gitdir:")
                        .map(|p| p.trim().to_string())
                })
                .map(|p| {
                    let p = PathBuf::from(p);
                    if p.is_absolute() {
                        p
                    } else {
                        dir.join(p)
                    }
                });
            return GitContext {
                root: Some(dir.to_string_lossy().into_owned()),
                branch: gitdir.as_deref().and_then(read_head),
            };
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => return GitContext::default(),
        }
    }
}

fn read_head(gitdir: &Path) -> Option<String> {
    let head = fs::read_to_string(gitdir.join("HEAD")).ok()?;
    let head = head.trim();
    if let Some(branch) = head.strip_prefix("ref: refs/heads/") {
        Some(branch.to_string())
    } else if head.len() >= 8 {
        // Detached HEAD: store the short hash.
        Some(head[..8].to_string())
    } else {
        None
    }
}

/// Tags are the words of a trailing comment: `cargo build # release deploy`
/// stores tags `release deploy`. Only simple word-like tokens count, so
/// ordinary comments with punctuation are not misread as tags.
pub fn extract_tags(command: &str) -> Vec<String> {
    let Some(pos) = command.rfind(" #") else {
        return Vec::new();
    };
    let after = &command[pos + 2..];
    let tags: Vec<String> = after.split_whitespace().map(str::to_string).collect();
    let word_like = |t: &String| {
        t.chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    };
    if !tags.is_empty() && tags.iter().all(word_like) {
        tags
    } else {
        Vec::new()
    }
}

/// Record the start of a command (called from zsh `preexec`).
/// Commands starting with a space are not recorded at all — the conventional
/// privacy escape hatch.
pub fn record_start(db: &Db, session: &str, raw_command: &str) -> Result<()> {
    if raw_command.trim().is_empty() || raw_command.starts_with(' ') {
        return Ok(());
    }
    let command = redact::redact(raw_command.trim_end());
    let tags = extract_tags(&command).join(" ");
    let cwd = std::env::current_dir()?;
    let git = git_context(&cwd);
    db.record_start(&NewEntry {
        command,
        cwd: cwd.to_string_lossy().into_owned(),
        git_repo: git.root,
        git_branch: git.branch,
        started_at: now_ms(),
        tags,
        session_id: session.to_string(),
    })?;
    Ok(())
}

/// Record the completion of a command (called from zsh `precmd`).
pub fn record_end(db: &Db, session: &str, exit_code: i32) -> Result<()> {
    db.record_end(session, exit_code, now_ms())?;
    Ok(())
}
