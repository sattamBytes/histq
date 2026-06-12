//! `histq pick`: a minimal full-screen picker for Ctrl+R.
//!
//! Renders on the alternate screen via stderr; the chosen command is printed
//! to stdout so the zsh widget can capture it with `$( ... )`. Type to
//! refine the query (results re-rank live), ↑/↓ to move, Enter to accept,
//! Esc to cancel.

use std::io::{stderr, Write};

use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    queue,
    style::{Attribute, Print, SetAttribute},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};

use crate::config::Config;
use crate::db::{Candidate, Db};
use crate::search::{self, RankContext};

const VISIBLE_ROWS: usize = 15;

/// Restores the terminal even if drawing or querying errors out.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode()?;
        let mut err = stderr();
        queue!(err, EnterAlternateScreen, cursor::Hide)?;
        err.flush()?;
        Ok(TerminalGuard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let mut err = stderr();
        let _ = queue!(err, cursor::Show, LeaveAlternateScreen);
        let _ = err.flush();
        let _ = terminal::disable_raw_mode();
    }
}

pub fn pick(db: &Db, config: &Config, initial_query: &str) -> Result<Option<String>> {
    let mut query = initial_query.to_string();
    let mut selected: usize = 0;

    let _guard = TerminalGuard::enter()?;

    loop {
        let results = run_query(db, config, &query)?;
        if selected >= results.len() {
            selected = results.len().saturating_sub(1);
        }
        draw(&query, &results, selected)?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Esc => return Ok(None),
                KeyCode::Char('c' | 'g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(None)
                }
                KeyCode::Enter => {
                    return Ok(results.get(selected).map(|c| c.command.clone()));
                }
                KeyCode::Up => selected = selected.saturating_sub(1),
                KeyCode::Down => {
                    if selected + 1 < results.len() {
                        selected += 1;
                    }
                }
                KeyCode::Backspace => {
                    query.pop();
                    selected = 0;
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    query.clear();
                    selected = 0;
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    query.push(c);
                    selected = 0;
                }
                _ => {}
            }
        }
    }
}

fn run_query(db: &Db, config: &Config, query: &str) -> Result<Vec<Candidate>> {
    let parsed = search::parse_query(query, chrono::Local::now());
    let candidates = db.candidates(&parsed, config.candidate_limit, true)?;
    let cwd = std::env::current_dir().unwrap_or_default();
    let git = crate::history::git_context(&cwd);
    let ctx = RankContext {
        cwd: cwd.to_string_lossy().into_owned(),
        git_repo: git.root,
        now_ms: crate::history::now_ms(),
        query_tags: parsed.tags,
    };
    let mut ranked = search::rank(candidates, &ctx, &config.weights);
    ranked.truncate(VISIBLE_ROWS);
    Ok(ranked)
}

fn draw(query: &str, results: &[Candidate], selected: usize) -> Result<()> {
    let mut err = stderr();
    let (cols, _) = terminal::size().unwrap_or((80, 24));
    let width = cols as usize;
    let now_ms = crate::history::now_ms();

    queue!(err, Clear(ClearType::All), cursor::MoveTo(0, 0))?;
    queue!(
        err,
        SetAttribute(Attribute::Bold),
        Print(format!("histq ❯ {query}")),
        SetAttribute(Attribute::Reset),
        Print("\r\n"),
        SetAttribute(Attribute::Dim),
        Print("↑/↓ move · enter accept · esc cancel"),
        SetAttribute(Attribute::Reset),
        Print("\r\n\r\n"),
    )?;

    if results.is_empty() {
        queue!(
            err,
            SetAttribute(Attribute::Dim),
            Print("no matches"),
            SetAttribute(Attribute::Reset)
        )?;
    }

    for (i, c) in results.iter().enumerate() {
        let marker = if i == selected { "❯ " } else { "  " };
        let status = match c.exit_code {
            Some(0) => "✓",
            Some(_) => "✗",
            None => "?",
        };
        let line = format!("{marker}{status} {}", c.command.replace('\n', " ⏎ "));
        let line = truncate_display(&line, width.saturating_sub(14));
        if i == selected {
            queue!(err, SetAttribute(Attribute::Reverse))?;
        }
        queue!(err, Print(&line))?;
        queue!(
            err,
            SetAttribute(Attribute::Dim),
            Print(format!("  {}", rel_time(now_ms, c.started_at))),
            SetAttribute(Attribute::Reset),
            Print("\r\n")
        )?;
    }
    err.flush()?;
    Ok(())
}

fn truncate_display(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn rel_time(now_ms: i64, then_ms: i64) -> String {
    let secs = ((now_ms - then_ms).max(0)) / 1000;
    match secs {
        0..=59 => format!("{secs}s ago"),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86_399 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}
