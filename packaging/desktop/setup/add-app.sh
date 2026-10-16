#!/usr/bin/env bash
# Makes a program show up in the app list of DMS (and any launcher that reads .desktop files).
#   add-app.sh --name "Lazygit" --exec "lazygit" [--terminal] [--icon NAME] [--categories "Development;"]
# --terminal runs the command inside foot. Writes ~/.local/share/applications/<name>.desktop.
set -euo pipefail
name=""; exec_cmd=""; icon="application-x-executable"; cats="Utility;"; term=0
while [ $# -gt 0 ]; do
  case "$1" in
    --name) name="$2"; shift 2 ;;
    --exec) exec_cmd="$2"; shift 2 ;;
    --icon) icon="$2"; shift 2 ;;
    --categories) cats="$2"; shift 2 ;;
    --terminal) term=1; shift ;;
    *) echo "unknown option $1" >&2; exit 1 ;;
  esac
done
[ -n "$name" ] && [ -n "$exec_cmd" ] || { sed -n 2,5p "$0"; exit 1; }
[ "$term" -eq 1 ] && exec_cmd="foot -e $exec_cmd"
slug=$(printf '%s' "$name" | tr 'A-Z ' 'a-z-' | tr -cd 'a-z0-9-')
dir="$HOME/.local/share/applications"; mkdir -p "$dir"
cat > "$dir/desktop-installer-$slug.desktop" <<EOT
[Desktop Entry]
Type=Application
Name=$name
Exec=$exec_cmd
Icon=$icon
Categories=$cats
Terminal=false
EOT
command -v update-desktop-database >/dev/null && update-desktop-database "$dir" 2>/dev/null || true
echo "added $name"
