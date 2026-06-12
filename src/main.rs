use std::io::IsTerminal;

use anyhow::{bail, Result};
use chrono::TimeZone;
use clap::{Parser, Subcommand};

use histq::db::{Candidate, Db};
use histq::search::{self, RankContext};
use histq::{history, shell};

#[derive(Parser)]
#[command(name = "histq", version, about = "Project-aware shell history for zsh")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print shell integration code (supported: zsh)
    Init { shell: String },
    /// Record the start of a command (called from zsh preexec)
    RecordStart {
        #[arg(long)]
        session: String,
        /// The command line as typed
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Record the completion of the pending command (called from zsh precmd)
    RecordEnd {
        #[arg(long)]
        session: String,
        #[arg(long)]
        exit_code: i32,
    },
    /// Print the Nth-best match for a query (used by the up-arrow widget)
    Previous {
        #[arg(long, default_value = "")]
        query: String,
        #[arg(long, default_value_t = 0)]
        offset: usize,
    },
    /// Print the Nth-best match from the same result set (down-arrow widget)
    Next {
        #[arg(long, default_value = "")]
        query: String,
        #[arg(long, default_value_t = 0)]
        offset: usize,
    },
    /// Search history, best matches first
    Search {
        #[arg(long, default_value_t = 20)]
        limit: usize,
        query: Vec<String>,
    },
    /// Show history in chronological order, newest first
    Timeline {
        #[arg(long, default_value_t = 50)]
        limit: usize,
        query: Vec<String>,
    },
}

const CANDIDATE_LIMIT: usize = 500;

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Init { shell } => match shell.as_str() {
            "zsh" => {
                print!("{}", shell::zsh::SCRIPT);
                Ok(())
            }
            other => bail!("unsupported shell {other:?} (supported: zsh)"),
        },
        Cmd::RecordStart { session, command } => {
            let db = Db::open_default()?;
            history::record_start(&db, &session, &command.join(" "))
        }
        Cmd::RecordEnd { session, exit_code } => {
            let db = Db::open_default()?;
            history::record_end(&db, &session, exit_code)
        }
        Cmd::Previous { query, offset } | Cmd::Next { query, offset } => nth_result(&query, offset),
        Cmd::Search { limit, query } => run_search(&query.join(" "), limit),
        Cmd::Timeline { limit, query } => run_timeline(&query.join(" "), limit),
    }
}

fn rank_context(query_tags: Vec<String>) -> Result<RankContext> {
    let cwd = std::env::current_dir()?;
    let git = history::git_context(&cwd);
    Ok(RankContext {
        cwd: cwd.to_string_lossy().into_owned(),
        git_repo: git.root,
        now_ms: history::now_ms(),
        query_tags,
    })
}

/// Shared by `previous` and `next`: print the offset-th ranked result, or
/// exit 1 with no output so the zsh widget knows to beep.
fn nth_result(query: &str, offset: usize) -> Result<()> {
    let db = Db::open_default()?;
    let parsed = search::parse_query(query, chrono::Local::now());
    let candidates = db.candidates(&parsed, CANDIDATE_LIMIT, true)?;
    let ctx = rank_context(parsed.tags.clone())?;
    let ranked = search::rank(candidates, &ctx);
    match ranked.get(offset) {
        Some(c) => {
            println!("{}", c.command);
            Ok(())
        }
        None => std::process::exit(1),
    }
}

fn run_search(query: &str, limit: usize) -> Result<()> {
    let db = Db::open_default()?;
    let parsed = search::parse_query(query, chrono::Local::now());
    let candidates = db.candidates(&parsed, CANDIDATE_LIMIT, false)?;
    let ctx = rank_context(parsed.tags.clone())?;
    let ranked = search::rank(candidates, &ctx);
    let now_ms = ctx.now_ms;

    for c in ranked.iter().take(limit) {
        println!("{}", c.command);
        println!("{}", dim(&format!("    {}", context_line(c, now_ms))));
    }
    Ok(())
}

fn run_timeline(query: &str, limit: usize) -> Result<()> {
    let db = Db::open_default()?;
    let parsed = search::parse_query(query, chrono::Local::now());
    let mut candidates = db.candidates(&parsed, CANDIDATE_LIMIT, false)?;
    candidates.sort_by_key(|c| std::cmp::Reverse(c.started_at));

    for c in candidates.iter().take(limit) {
        let when = chrono::Local
            .timestamp_millis_opt(c.started_at)
            .single()
            .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "????-??-?? ??:??:??".into());
        let status = exit_marker(c.exit_code);
        let duration = c.duration_ms.map(fmt_duration).unwrap_or_default();
        println!("{when}  {status} {duration:>8}  {}", c.command);
    }
    Ok(())
}

fn context_line(c: &Candidate, now_ms: i64) -> String {
    let mut parts = vec![shorten_home(&c.cwd)];
    if let Some(branch) = &c.git_branch {
        parts.push(branch.clone());
    }
    parts.push(match c.exit_code {
        Some(0) => "ok".into(),
        Some(code) => format!("exit {code}"),
        None => "?".into(),
    });
    parts.push(rel_time(now_ms, c.started_at));
    if let Some(d) = c.duration_ms {
        parts.push(fmt_duration(d));
    }
    parts.join(" · ")
}

fn exit_marker(exit_code: Option<i32>) -> &'static str {
    match exit_code {
        Some(0) => "✓",
        Some(_) => "✗",
        None => "?",
    }
}

fn shorten_home(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
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

fn fmt_duration(ms: i64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{}m{}s", ms / 60_000, (ms % 60_000) / 1000)
    }
}

fn dim(s: &str) -> String {
    if std::io::stdout().is_terminal() {
        format!("\x1b[2m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}
