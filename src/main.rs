use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::{bail, Result};
use chrono::TimeZone;
use clap::{Parser, Subcommand};

use histq::config::Config;
use histq::db::{Candidate, Db};
use histq::search::{self, RankContext};
use histq::{history, import, shell, tui};

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
    /// Backfill the database from an existing history file (~/.zsh_history)
    Import {
        /// History file to import (default: ~/.zsh_history)
        #[arg(long)]
        file: Option<PathBuf>,
    },
    /// Delete entries by id, or by substring with --contains
    Delete {
        /// Entry ids (shown in `histq timeline` / `--contains` listings)
        ids: Vec<i64>,
        /// Match entries containing this exact substring (not tokenized —
        /// works for partial secrets)
        #[arg(long)]
        contains: Option<String>,
        /// Actually delete --contains matches (without it, they are only listed)
        #[arg(long)]
        yes: bool,
    },
    /// Interactive full-screen picker (bound to Ctrl+R by `histq init zsh`)
    Pick {
        #[arg(long, default_value = "")]
        query: String,
    },
    /// Usage statistics: top commands, failure rates, busiest hours
    Stats {
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
}

fn main() -> Result<()> {
    let config = Config::load()?;
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
            let extra = config.extra_redact_patterns()?;
            history::record_start(&db, &session, &command.join(" "), &extra)
        }
        Cmd::RecordEnd { session, exit_code } => {
            let db = Db::open_default()?;
            history::record_end(&db, &session, exit_code)
        }
        Cmd::Previous { query, offset } | Cmd::Next { query, offset } => {
            nth_result(&config, &query, offset)
        }
        Cmd::Search { limit, query } => run_search(&config, &query.join(" "), limit),
        Cmd::Timeline { limit, query } => run_timeline(&config, &query.join(" "), limit),
        Cmd::Import { file } => run_import(&config, file),
        Cmd::Delete { ids, contains, yes } => run_delete(ids, contains, yes),
        Cmd::Pick { query } => {
            let db = Db::open_default()?;
            match tui::pick(&db, &config, &query)? {
                Some(command) => {
                    println!("{command}");
                    Ok(())
                }
                None => std::process::exit(1),
            }
        }
        Cmd::Stats { limit } => run_stats(limit),
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
fn nth_result(config: &Config, query: &str, offset: usize) -> Result<()> {
    let db = Db::open_default()?;
    let parsed = search::parse_query(query, chrono::Local::now());
    let candidates = db.candidates(&parsed, config.candidate_limit, true)?;
    let ctx = rank_context(parsed.tags.clone())?;
    let ranked = search::rank(candidates, &ctx, &config.weights);
    match ranked.get(offset) {
        Some(c) => {
            println!("{}", c.command);
            Ok(())
        }
        None => std::process::exit(1),
    }
}

fn run_search(config: &Config, query: &str, limit: usize) -> Result<()> {
    let db = Db::open_default()?;
    let parsed = search::parse_query(query, chrono::Local::now());
    let candidates = db.candidates(&parsed, config.candidate_limit, false)?;
    let ctx = rank_context(parsed.tags.clone())?;
    let ranked = search::rank(candidates, &ctx, &config.weights);
    let now_ms = ctx.now_ms;

    for c in ranked.iter().take(limit) {
        println!("{}", c.command);
        println!("{}", dim(&format!("    {}", context_line(c, now_ms))));
    }
    Ok(())
}

fn run_timeline(config: &Config, query: &str, limit: usize) -> Result<()> {
    let db = Db::open_default()?;
    let parsed = search::parse_query(query, chrono::Local::now());
    let mut candidates = db.candidates(&parsed, config.candidate_limit, false)?;
    candidates.sort_by_key(|c| std::cmp::Reverse(c.started_at));

    for c in candidates.iter().take(limit) {
        println!("{}", timeline_line(c));
    }
    Ok(())
}

fn run_import(config: &Config, file: Option<PathBuf>) -> Result<()> {
    let path = match file {
        Some(p) => p,
        None => PathBuf::from(std::env::var("HOME")?).join(".zsh_history"),
    };
    let db = Db::open_default()?;
    let extra = config.extra_redact_patterns()?;
    let report = import::import_file(
        &db,
        &path,
        |cmd| histq::redact::redact_with(cmd, &extra),
        history::now_ms(),
    )?;
    println!(
        "imported {} entries from {} ({} already present, skipped)",
        report.imported,
        path.display(),
        report.skipped
    );
    if report.imported > 0 {
        println!("note: imported entries have no directory/git/exit metadata");
    }
    Ok(())
}

fn run_delete(ids: Vec<i64>, contains: Option<String>, yes: bool) -> Result<()> {
    let db = Db::open_default()?;

    if ids.is_empty() && contains.is_none() {
        bail!("nothing to delete: pass ids (see `histq timeline`) or --contains TEXT");
    }

    let mut deleted = 0;
    if !ids.is_empty() {
        deleted += db.delete_ids(&ids)?;
    }

    if let Some(needle) = contains {
        let matches = db.find_containing(&needle, 1000)?;
        if matches.is_empty() {
            println!("no entries contain {needle:?}");
        } else if yes {
            let match_ids: Vec<i64> = matches.iter().map(|c| c.id).collect();
            deleted += db.delete_ids(&match_ids)?;
        } else {
            for c in &matches {
                println!("{}", timeline_line(c));
            }
            println!(
                "\n{} matching entries — re-run with --yes to delete them",
                matches.len()
            );
            return Ok(());
        }
    }

    println!("deleted {deleted} entries");
    Ok(())
}

fn run_stats(limit: usize) -> Result<()> {
    let db = Db::open_default()?;
    let stats = db.stats(limit)?;

    if stats.total == 0 {
        println!("no history yet");
        return Ok(());
    }

    let pct = |n: i64| 100.0 * n as f64 / stats.total as f64;
    println!("commands recorded : {}", stats.total);
    println!(
        "succeeded         : {} ({:.1}%)",
        stats.succeeded,
        pct(stats.succeeded)
    );
    println!(
        "failed            : {} ({:.1}%)",
        stats.failed,
        pct(stats.failed)
    );

    if !stats.top_commands.is_empty() {
        println!("\nmost used:");
        let max = stats.top_commands.iter().map(|r| r.1).max().unwrap_or(1);
        for (cmd, n, fails) in &stats.top_commands {
            let bar = bar(*n, max, 20);
            let fail_note = if *fails > 0 {
                format!("  ({fails} failed)")
            } else {
                String::new()
            };
            println!("  {n:>5}  {bar}  {}{fail_note}", oneline(cmd, 60));
        }
    }

    if !stats.failing.is_empty() {
        println!("\nmost failing (3+ runs):");
        for (cmd, n, fails) in &stats.failing {
            println!(
                "  {:>4.0}%  {fails}/{n}  {}",
                100.0 * *fails as f64 / *n as f64,
                oneline(cmd, 60)
            );
        }
    }

    if !stats.top_dirs.is_empty() {
        println!("\nbusiest directories:");
        for (dir, n, _) in &stats.top_dirs {
            println!("  {n:>5}  {}", shorten_home(dir));
        }
    }

    let hour_max = stats.by_hour.iter().copied().max().unwrap_or(0);
    if hour_max > 0 {
        println!("\nby hour of day:");
        for (hour, n) in stats.by_hour.iter().enumerate() {
            if *n > 0 {
                println!("  {hour:>2}:00  {}  {n}", bar(*n, hour_max, 30));
            }
        }
    }
    Ok(())
}

fn bar(n: i64, max: i64, width: usize) -> String {
    let filled = ((n as f64 / max as f64) * width as f64).round() as usize;
    format!(
        "{}{}",
        "█".repeat(filled.max(1)),
        " ".repeat(width - filled.max(1).min(width))
    )
}

fn oneline(s: &str, max_chars: usize) -> String {
    let s = s.replace('\n', " ⏎ ");
    if s.chars().count() <= max_chars {
        s
    } else {
        let mut out: String = s.chars().take(max_chars - 1).collect();
        out.push('…');
        out
    }
}

fn timeline_line(c: &Candidate) -> String {
    let when = chrono::Local
        .timestamp_millis_opt(c.started_at)
        .single()
        .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "????-??-?? ??:??:??".into());
    let status = exit_marker(c.exit_code);
    let duration = c.duration_ms.map(fmt_duration).unwrap_or_default();
    format!(
        "{:>6}  {when}  {status} {duration:>8}  {}",
        c.id,
        oneline(&c.command, 100)
    )
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
