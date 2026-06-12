# histq plugin for zsh plugin managers (oh-my-zsh, zinit, antidote, ...).
# This only wires up the shell integration — the histq binary itself must be
# installed separately: https://github.com/sattamBytes/histq#installation

if (( $+commands[histq] )); then
  eval "$(histq init zsh)"
else
  print -u2 "histq: binary not found on \$PATH; see https://github.com/sattamBytes/histq#installation"
fi
