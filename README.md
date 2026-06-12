# histq

**Project-aware shell history for zsh: an up-arrow that knows your repo.**

histq replaces zsh's up/down-arrow history with a richer, context-aware system.
Every command you run is recorded to a local SQLite database along with where
you ran it (directory, git repo, branch), when, how long it took, and whether
it succeeded. When you press the up arrow, histq ranks your history so that
commands from **this repo**, run **recently**, that **worked**, come up first —
and whatever you've already typed on the line is used as the search query.

No AI, no cloud, no telemetry. Structured metadata, SQLite full-text search,
simple keyword rules, and a hand-written ranking function. Everything stays on
your machine.

```
$ git ch▌            # press ↑
$ git checkout -b fix/login-redirect▌   # most relevant first: this repo, recent, succeeded
                     # press ↑ again to go deeper, ↓ to come back
```

## How it compares

[atuin](https://github.com/atuinsh/atuin) and
[mcfly](https://github.com/cantino/mcfly) cover similar ground. histq differs
in three deliberate ways:

- **Inline buffer replacement, not a TUI.** Up-arrow swaps the command line in
  place via ZLE — the native shell feel, no full-screen picker.
- **Git-repo-aware ranking by default.** "Same repo" is a core ranking signal,
  not an opt-in filter.
- **Write-time secret redaction.** Tokens and passwords are scrubbed *before*
  they touch disk, not hidden at display time.

## Installation

### Homebrew (macOS / Linux)

```sh
brew install sattamBytes/tap/histq
```

### Shell installer (macOS / Linux, no dependencies)

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sattamBytes/histq/releases/latest/download/histq-installer.sh | sh
```

### Cargo (requires Rust 1.80+)

```sh
cargo install histq
# or from a checkout:
cargo install --path .
```

Prebuilt binaries for Apple Silicon / Intel macOS and x86_64 / aarch64 Linux
are attached to every [GitHub release](https://github.com/sattamBytes/histq/releases).

### Enable the zsh integration

Whichever way you installed the binary, add one line to `~/.zshrc`:

```sh
eval "$(histq init zsh)"
```

Or, with a zsh plugin manager (the binary still needs to be installed first):

```sh
zinit load sattamBytes/histq        # zinit
antidote install sattamBytes/histq  # antidote
```

Open a new shell and start typing. The database lives at
`~/.local/share/histq/history.db` (override with `$HISTQ_DB`).

Bring your years of accumulated history along:

```sh
histq import                      # backfills from ~/.zsh_history
histq import --file ~/.bash_history
```

Both zsh formats are handled (extended history keeps its timestamps; plain
entries import without them). Importing is idempotent — re-running it never
duplicates. Imported entries carry no directory/git/exit metadata, so live
recorded history naturally outranks them over time.

## Usage

### Arrow keys

- **↑** — if the line is empty, step through your best-ranked recent commands.
  If you've typed something, it becomes the query: `docker ↑` finds your best
  docker commands, preferring this repo, recent, and successful ones.
- **↑ ↑ ↑** — step deeper into the ranked results.
- **↓** — step back toward what you originally typed.
- **Ctrl+R** — full-screen picker for visual scanning: type to refine the
  query (results re-rank live, same rule words as `search`), ↑/↓ to move,
  Enter to put the selection on your command line, Esc to cancel.
- When there are no (more) results, the terminal beeps and the line is left alone.

### Searching

```sh
histq search docker failed            # docker commands that exited non-zero
histq search "cargo" worked today     # successful cargo commands since midnight
histq search deploy yesterday         # deploy commands from yesterday
histq search migrate last week        # the past 7 days
histq search #release                 # commands tagged "release"
histq timeline                        # chronological view, newest first
histq timeline npm --limit 100
```

Rule words are plain English, parsed with simple rules (no AI):

| Word(s) | Meaning |
|---|---|
| `failed`, `error`, `broke` | exit code ≠ 0 |
| `worked`, `success`, `passed` | exit code = 0 |
| `today` / `yesterday` / `last week` | timestamp range |
| `#tag` | tag filter |
| anything else | full-text search (last word matches as a prefix) |

### Tags

End a command with a comment and the words become tags:

```sh
cargo publish  # release
histq search #release
```

### Stats

```sh
histq stats
```

Shows totals and success rate, your most-used commands (with bars and failure
counts), the commands that fail most often, busiest directories, and an
hour-of-day histogram.

### Privacy

- Everything is local: one SQLite file, no network access, ever.
- Secrets are redacted **before storage**: `Authorization` headers,
  `key=value` assignments for sensitive key names (`API_KEY`, `PASSWORD`,
  `TOKEN`, ...), `--password`-style flags, and known token shapes (AWS,
  GitHub, Slack, JWT) are replaced with `***REDACTED***`. Add your own
  patterns in the config file (below).
- Start a command with a **space** and it is not recorded at all.
- Something slipped through anyway? Delete it:

  ```sh
  histq delete --contains "oops-secret"        # lists matches with their ids
  histq delete --contains "oops-secret" --yes  # actually deletes them
  histq delete 1234 1235                       # or by id from `histq timeline`
  ```

### Configuration (optional)

Everything works with no config. To tune behavior, create
`~/.config/histq/config.toml` (or point `$HISTQ_CONFIG` at a file):

```toml
# how many rows SQLite hands to the ranking pass
candidate_limit = 500

# ranking weights — e.g. bias hard toward recency, ignore directory:
[weights]
text = 4.0
repo = 2.0
cwd = 1.5
recency = 1.0
success = 0.5
tags = 1.0
recency_half_life_days = 7.0

# your organization's internal token shapes, redacted before storage:
[redact]
extra_patterns = ['mycorp_[A-Za-z0-9]{32}', 'internal-token-[0-9a-f]+']
```

All fields are optional; unknown keys are rejected loudly so typos can't
silently change behavior.

## Commands

| Command | Purpose |
|---|---|
| `histq init zsh` | print the zsh integration script |
| `histq record-start --session S -- CMD` | record a command starting (called by `preexec`) |
| `histq record-end --session S --exit-code N` | attach exit code + duration (called by `precmd`) |
| `histq previous --query Q --offset N` | print the Nth-best match (up-arrow widget) |
| `histq next --query Q --offset N` | same result set, used by the down-arrow widget |
| `histq search [QUERY...]` | ranked search |
| `histq timeline [QUERY...]` | chronological history (shows entry ids) |
| `histq import [--file F]` | backfill from an existing history file |
| `histq delete IDS... / --contains TEXT --yes` | remove entries |
| `histq pick [--query Q]` | interactive picker (the Ctrl+R widget) |
| `histq stats [--limit N]` | usage statistics |

## Architecture

```
src/
├── main.rs       CLI entrypoint (clap), output formatting
├── config.rs     optional config.toml (weights, limits, extra redaction)
├── db.rs         SQLite schema + queries (rusqlite, FTS5, WAL)
├── history.rs    record-start/record-end, git context, tag extraction
├── import.rs     ~/.zsh_history parsing + idempotent backfill
├── search.rs     query parsing (rule words) + ranking
├── redact.rs     secret redaction patterns
├── tui.rs        the Ctrl+R picker (crossterm, alternate screen)
└── shell/
    └── zsh.rs    the script printed by `histq init zsh`
```

**Recording.** zsh's `preexec` hook calls `record-start` (inserts the command
with cwd, git repo root, branch, and timestamp); `precmd` calls `record-end`
(fills in exit code and duration). Each terminal tab has its own session id,
so concurrent shells never clobber each other.

**Storage.** One `commands` table plus an FTS5 index kept in sync by triggers.
WAL mode so readers (your keypresses) never block on writers (other tabs).

**Search.** The query is parsed by rules: filter words become SQL filters,
the rest becomes an FTS5 match expression. Candidates (≤500) are fetched by
SQL, then ranked in Rust:

```
score = 4.0 · text match        (normalized bm25)
      + 2.0 · same git repo
      + 1.5 · same directory
      + 1.0 · recency            (exponential decay, 7-day half-life)
      + 0.5 · exited 0
      + 1.0 · tag overlap
```

(All weights configurable via `[weights]` in the config file.)

Duplicate command texts are collapsed, keeping the best-scored instance.

**The arrow keys.** ZLE widgets bound to `↑`/`↓` call `histq previous`/`next`
with the original line as the query and an offset the widget tracks in shell
variables; the result replaces the line via `BUFFER`/`CURSOR`. The binary is
stateless, so it stays a few milliseconds per keypress: no `git` subprocess
(the repo root and branch are discovered by walking the filesystem and reading
`.git/HEAD` directly), one indexed SQLite query, done. Measured: ~5ms per
keypress against a few-thousand-row database, including process startup.

## Development

```sh
cargo test          # unit + integration tests (storage, ranking, parsing, redaction, CLI)
cargo build --release
```

CI runs fmt, clippy, and tests on Ubuntu and macOS for every push.

**Releasing:** push a tag like `v0.1.0` and the [release workflow](.github/workflows/release.yml)
([cargo-dist](https://github.com/axodotdev/cargo-dist)) builds binaries for all
four targets, creates the GitHub release with the shell installer, and pushes
the Homebrew formula to the tap. One-time setup: create the
`sattamBytes/homebrew-tap` repo and add a `HOMEBREW_TAP_TOKEN` repo secret
(a fine-grained PAT with write access to the tap).

## License

MIT
