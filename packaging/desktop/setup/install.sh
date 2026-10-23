#!/usr/bin/env bash
# Installs the packages chosen in the wizard. Idempotent: safe to re-run.
#
#   ./install.sh <selection-file>   install the ids listed (one per line) plus all base items
#   ./install.sh --check            verify that every apt package in the manifest exists
#   ./install.sh --list             print the manifest ids by layer
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MANIFEST="$DIR/packages.tsv"
SUDO=""; [ "$(id -u)" -eq 0 ] || SUDO="sudo"
export INSTALLER_DIR="$DIR"

# Prints manifest rows without comments or blank lines.
rows() { grep -vE '^\s*(#|$)' "$MANIFEST"; }

case "${1:-}" in
  --list)
    rows | awk -F'\t' '{printf "%-10s %-14s %s\n", $2, $1, $5}'
    exit 0 ;;
  --check)
    missing=0
    while IFS=$'\t' read -r id layer method target _; do
      [ "$method" = apt ] || continue
      for pkg in $target; do
        if ! apt-cache show "$pkg" >/dev/null 2>&1; then
          echo "MISSING  $id: $pkg"; missing=1
        fi
      done
    done < <(rows)
    [ "$missing" -eq 0 ] && echo "All apt packages found."
    exit "$missing" ;;
  ""|-h|--help)
    sed -n '2,7p' "$0"; exit 1 ;;
esac

SELECTION="$1"
[ -r "$SELECTION" ] || { echo "Cannot read selection file: $SELECTION" >&2; exit 1; }

apt_pkgs=(); flatpaks=(); mise_tools=(); scripts=(); selected=(); fallbacks=()
while IFS=$'\t' read -r id layer method target label _ _ fallback; do
  if [ "$layer" != base ] && ! grep -qxF "$id" "$SELECTION"; then continue; fi
  selected+=("$id")
  case "$method" in
    apt)     for p in $target; do apt_pkgs+=("$p"); done
             # an apt row may name a Flathub app (flatpak:<id>) for systems whose archive does not have the package (Ubuntu has no Chromium deb)
             case "$fallback" in flatpak:*) fallbacks+=("$target ${fallback#flatpak:}") ;; esac ;;
    flatpak) flatpaks+=("$target") ;;
    mise)    mise_tools+=("$target") ;;
    script)  scripts+=("$label	$target") ;;
  esac
done < <(rows)

# Flatpak itself is needed as soon as one Flatpak app is selected.
[ "${#flatpaks[@]}" -gt 0 ] && apt_pkgs+=(flatpak)
# mise tools and the scripts in the manifest all need curl.
apt_pkgs+=(curl ca-certificates)

echo "==> apt"
$SUDO apt-get update
# What the archive of this system does not have goes to Flathub instead, where the row says how.
for fb in "${fallbacks[@]}"; do
  app=${fb##* }; pkgs=${fb% *}; missing=""
  for p in $pkgs; do
    apt-cache policy "$p" 2>/dev/null | awk '/Candidate:/ { found = ($2 != "(none)") } END { exit !found }' || missing=1
  done
  if [ -n "$missing" ]; then
    echo "no $pkgs in this system's archive: $app from Flathub instead"
    for p in $pkgs; do for i in "${!apt_pkgs[@]}"; do [ "${apt_pkgs[$i]}" = "$p" ] && unset 'apt_pkgs[i]'; done; done
    apt_pkgs=("${apt_pkgs[@]}")
    flatpaks+=("$app")
    case " ${apt_pkgs[*]} " in *" flatpak "*) ;; *) apt_pkgs+=(flatpak) ;; esac
  fi
done
$SUDO apt-get install -y --no-install-recommends "${apt_pkgs[@]}"

if [ "${#flatpaks[@]}" -gt 0 ]; then
  echo "==> flatpak"
  $SUDO flatpak remote-add --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
  for app in "${flatpaks[@]}"; do
    if flatpak info "$app" >/dev/null 2>&1; then echo "have $app"; continue; fi
    $SUDO flatpak install -y --noninteractive flathub "$app" || echo "FAILED flatpak: $app"
  done
fi

export PATH="$HOME/.local/bin:$PATH"

if [ "${#mise_tools[@]}" -gt 0 ]; then
  echo "==> mise"
  for t in "${mise_tools[@]}"; do mise use -g "$t" || echo "FAILED mise: $t"; done
fi

if [ "${#scripts[@]}" -gt 0 ]; then
  echo "==> scripts"
  for entry in "${scripts[@]}"; do
    label="${entry%%$'\t'*}"; spec="${entry#*$'\t'}"
    bin="${spec%%::*}"; cmd="${spec#*::}"
    if command -v "$bin" >/dev/null 2>&1; then echo "have $label"; continue; fi
    echo "installing $label"
    bash -c "$cmd" || echo "FAILED script: $label"
  done
fi

echo "==> app list entries"
# Programs without a .desktop file of their own get one, so they show up in the DMS app list.
while IFS=$'\t' read -r id name cmd icon cats term; do
  printf '%s\n' "${selected[@]}" | grep -qxF "$id" || continue
  command -v "${cmd%% *}" >/dev/null 2>&1 || { echo "skip $name (not installed)"; continue; }
  args=(--name "$name" --exec "$cmd" --icon "$icon" --categories "$cats")
  [ "$term" = yes ] && args+=(--terminal)
  "$DIR/add-app.sh" "${args[@]}"
done < <(grep -vE '^\s*(#|$)' "$DIR/desktop-entries.tsv")

# Flatpak apps are listed only if their export folders are in XDG_DATA_DIRS when the session starts.
if ! grep -qF '# >>> desktop installer: xdg' "$HOME/.profile" 2>/dev/null; then
  block='# >>> desktop installer: xdg
for d in /var/lib/flatpak/exports/share "$HOME/.local/share/flatpak/exports/share"; do
  case ":${XDG_DATA_DIRS:-/usr/local/share:/usr/share}:" in
    *":$d:"*) ;;
    *) [ -d "$d" ] && XDG_DATA_DIRS="${XDG_DATA_DIRS:-/usr/local/share:/usr/share}:$d" ;;
  esac
done
export XDG_DATA_DIRS
# <<< desktop installer: xdg'
  { printf '%s\n' "$block"; cat "$HOME/.profile" 2>/dev/null; } > "$HOME/.profile.new" && mv "$HOME/.profile.new" "$HOME/.profile"
fi

echo "==> shell setup"
mkdir -p "$HOME/.config/desktop"
cp "$DIR/shell.sh" "$HOME/.config/desktop/shell.sh"
if ! grep -qF '.config/desktop/shell.sh' "$HOME/.bashrc" 2>/dev/null; then
  printf '\n# desktop installer shell setup\n[ -r ~/.config/desktop/shell.sh ] && . ~/.config/desktop/shell.sh\n' >> "$HOME/.bashrc"
fi

echo "Done. Log out and in (or open a new terminal) so PATH changes apply."
