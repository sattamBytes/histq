//! The zsh integration script printed by `histq init zsh`.

pub const SCRIPT: &str = r#"# histq zsh integration
# Install by adding this line to ~/.zshrc:
#   eval "$(histq init zsh)"
# Requires the `histq` binary on $PATH.

typeset -g HISTQ_SESSION="$$-$RANDOM$RANDOM"
typeset -g _HISTQ_ACTIVE=""
typeset -g _HISTQ_QUERY=""
typeset -g _HISTQ_SAVED=""
typeset -gi _HISTQ_OFFSET=-1

# preexec: the command is about to run — record it with its context.
_histq_preexec() {
  histq record-start --session "$HISTQ_SESSION" -- "$1" 2>/dev/null
}

# precmd: the command finished — attach exit code and duration.
_histq_precmd() {
  local exit_code=$?
  histq record-end --session "$HISTQ_SESSION" --exit-code $exit_code 2>/dev/null
  _HISTQ_ACTIVE=""
  _HISTQ_OFFSET=-1
}

autoload -Uz add-zsh-hook
add-zsh-hook preexec _histq_preexec
add-zsh-hook precmd _histq_precmd

# Up arrow: whatever is on the line becomes the query; each further press
# steps deeper into the ranked results.
_histq_up() {
  if [[ -z $_HISTQ_ACTIVE ]]; then
    _HISTQ_ACTIVE=1
    _HISTQ_QUERY="$BUFFER"
    _HISTQ_SAVED="$BUFFER"
    _HISTQ_OFFSET=-1
  fi
  local result
  if result=$(histq previous --query "$_HISTQ_QUERY" --offset $(( _HISTQ_OFFSET + 1 )) 2>/dev/null); then
    (( _HISTQ_OFFSET += 1 ))
    BUFFER="$result"
    CURSOR=$#BUFFER
  else
    zle beep
  fi
}

# Down arrow: step back toward the line you originally typed.
_histq_down() {
  if [[ -z $_HISTQ_ACTIVE ]]; then
    zle beep
    return
  fi
  if (( _HISTQ_OFFSET <= 0 )); then
    BUFFER="$_HISTQ_SAVED"
    CURSOR=$#BUFFER
    _HISTQ_ACTIVE=""
    _HISTQ_OFFSET=-1
    return
  fi
  (( _HISTQ_OFFSET -= 1 ))
  local result
  if result=$(histq next --query "$_HISTQ_QUERY" --offset $_HISTQ_OFFSET 2>/dev/null); then
    BUFFER="$result"
    CURSOR=$#BUFFER
  else
    zle beep
  fi
}

zle -N _histq_up
zle -N _histq_down
# Both CSI and SS3 arrow sequences, so it works across terminal modes.
bindkey '^[[A' _histq_up
bindkey '^[OA' _histq_up
bindkey '^[[B' _histq_down
bindkey '^[OB' _histq_down
"#;
