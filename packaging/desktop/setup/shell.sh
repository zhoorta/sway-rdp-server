# Shell setup for the remote desktop (bash). Sourced from ~/.bashrc by install.sh.
# Each tool is wired only if it is installed, so this is safe on a minimal system.

if command -v eza >/dev/null; then
  alias ls='eza --group-directories-first'
  alias ll='eza -l --git --group-directories-first'
  alias la='eza -la --git --group-directories-first'
else
  alias ls='ls --color=auto'
  alias ll='ls -l --color=auto'
  alias la='ls -la --color=auto'
fi
command -v batcat >/dev/null && alias cat='batcat --paging=never'
command -v fdfind >/dev/null && alias fd='fdfind'
alias ..='cd ..'
alias grep='grep --color=auto'

# fzf: Ctrl+R history, Ctrl+T files, Alt+C directories.
command -v fzf >/dev/null && eval "$(fzf --bash 2>/dev/null)"

command -v zoxide >/dev/null && eval "$(zoxide init bash)"
command -v direnv >/dev/null && eval "$(direnv hook bash)"
command -v mise >/dev/null && eval "$(mise activate bash)"
command -v starship >/dev/null && eval "$(starship init bash)"

export PATH="$HOME/.local/bin:$PATH"
