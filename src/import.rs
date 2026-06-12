//! Backfill the database from an existing shell history file.
//!
//! Supports both zsh formats:
//! - extended (`setopt EXTENDED_HISTORY`): `: <epoch>:<duration>;<command>`
//! - plain: one command per line (also covers ~/.bash_history)
//!
//! Multi-line entries (lines ending in `\`) are joined.

use std::path::Path;

use anyhow::{Context, Result};

use crate::db::Db;

#[derive(Debug, Clone, PartialEq)]
pub struct ImportedEntry {
    pub command: String,
    /// Unix epoch ms; None when the file has no timestamps (plain format).
    pub started_at: Option<i64>,
    pub duration_ms: Option<i64>,
}

/// Parse a history file's text into entries. Pure function, unit-testable.
pub fn parse_history(text: &str) -> Vec<ImportedEntry> {
    let mut entries = Vec::new();
    let mut acc = String::new();

    for raw in text.lines() {
        acc.push_str(raw);
        // zsh writes multi-line commands with backslash-newline continuations.
        if let Some(stripped) = acc.strip_suffix('\\') {
            let mut s = stripped.to_string();
            s.push('\n');
            acc = s;
            continue;
        }
        let line = std::mem::take(&mut acc);
        if let Some(entry) = parse_line(&line) {
            entries.push(entry);
        }
    }
    if !acc.is_empty() {
        if let Some(entry) = parse_line(&acc) {
            entries.push(entry);
        }
    }
    entries
}

fn parse_line(line: &str) -> Option<ImportedEntry> {
    // Extended format: `: 1612345678:0;command`
    if let Some(rest) = line.strip_prefix(": ") {
        if let Some((meta, command)) = rest.split_once(';') {
            if let Some((epoch, duration)) = meta.split_once(':') {
                if let (Ok(epoch), Ok(duration)) =
                    (epoch.trim().parse::<i64>(), duration.trim().parse::<i64>())
                {
                    let command = command.trim();
                    if command.is_empty() {
                        return None;
                    }
                    return Some(ImportedEntry {
                        command: command.to_string(),
                        started_at: Some(epoch * 1000),
                        duration_ms: Some(duration * 1000),
                    });
                }
            }
        }
    }
    // Plain format: the whole line is the command.
    let command = line.trim();
    if command.is_empty() {
        return None;
    }
    Some(ImportedEntry {
        command: command.to_string(),
        started_at: None,
        duration_ms: None,
    })
}

pub struct ImportReport {
    pub imported: usize,
    pub skipped: usize,
}

/// Import a history file. Idempotent: entries already imported (same command
/// and timestamp) are skipped, so re-running is safe. Commands are redacted
/// with the same rules as live recording. Entries without timestamps get a
/// fixed "30 days ago" timestamp so they rank below genuinely recent history.
pub fn import_file(
    db: &Db,
    path: &Path,
    redactor: impl Fn(&str) -> String,
    now_ms: i64,
) -> Result<ImportReport> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading history file {}", path.display()))?;
    let entries = parse_history(&text);

    const THIRTY_DAYS_MS: i64 = 30 * 86_400_000;
    let fallback_ts = now_ms - THIRTY_DAYS_MS;

    let rows: Vec<(String, Option<i64>, Option<i64>)> = entries
        .into_iter()
        .map(|e| (redactor(&e.command), e.started_at, e.duration_ms))
        .collect();

    let (imported, skipped) = db.import_entries(&rows, fallback_ts)?;
    Ok(ImportReport { imported, skipped })
}
