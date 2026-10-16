#!/usr/bin/env bash
# Install wizard: pick a preset, adjust layers and apps, confirm, then run install.sh.
# Uses whiptail (ships with Debian), so it works over SSH and in the first-boot console.
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MANIFEST="$DIR/packages.tsv"
SELECTION="${XDG_STATE_HOME:-$HOME/.local/state}/desktop-installer/selection"
mkdir -p "$(dirname "$SELECTION")"
command -v whiptail >/dev/null || { echo "whiptail missing: sudo apt install whiptail" >&2; exit 1; }

# GPU render node present? Without one, gpu-tagged apps start unticked and are flagged.
ARCH=$(uname -m)
HAS_GPU=0; ls /dev/dri/renderD* >/dev/null 2>&1 && HAS_GPU=1

rows() { grep -vE '^\s*(#|$)' "$MANIFEST"; }

# Runs whiptail and returns its output on stdout (whiptail draws on stdout, answers on stderr).
ui() { whiptail "$@" 3>&1 1>&2 2>&3; }

preset=$(ui --title "Remote desktop setup" --menu "What will this desktop be used for?" 17 70 5 \
  developer "Developer: shell, editors, containers, languages" \
  creative  "Creative: image, video, audio, 3D, notes" \
  both      "Developer and creative" \
  minimal   "Base only: browser, office, essentials" \
  custom    "Choose everything yourself") || exit 1

on_desktop=ON; on_dev=OFF; on_creative=OFF; on_ai=OFF
case "$preset" in
  developer) on_dev=ON; on_ai=ON ;;
  creative)  on_creative=ON ;;
  both)      on_dev=ON; on_creative=ON; on_ai=ON ;;
  minimal)   on_desktop=OFF ;;
esac

layers=$(ui --title "Layers" --checklist "Base is always installed. Pick extra layers (space to toggle):" 16 70 4 \
  desktop  "Desktop shell (DankMaterialShell)" "$on_desktop" \
  dev      "Developer tools"  "$on_dev" \
  creative "Creative apps"    "$on_creative" \
  ai       "AI tools"         "$on_ai") || exit 1
layers=$(tr -d '"' <<<"$layers")

: > "$SELECTION"

for layer in $layers; do
  items=()
  while IFS=$'\t' read -r id l method target label desc tags; do
    [ "$l" = "$layer" ] || continue
    [[ ",$tags," == *,x86only,* && "$ARCH" != x86_64 ]] && continue
    state=OFF; [[ ",$tags," == *,default,* ]] && state=ON
    if [[ ",$tags," == *,gpu,* ]]; then
      desc="$desc [needs GPU]"
      [ "$HAS_GPU" -eq 0 ] && state=OFF
    fi
    items+=("$id" "$label: $desc" "$state")
  done < <(rows)
  [ "${#items[@]}" -gt 0 ] || continue

  note=""; [ "$HAS_GPU" -eq 0 ] && note="No GPU detected, so GPU apps are unticked. "
  chosen=$(ui --title "$layer" --checklist "${note}Select apps to install:" 24 90 14 "${items[@]}") || exit 1
  tr -d '"' <<<"$chosen" | tr ' ' '\n' | grep -v '^$' >> "$SELECTION" || true
done

n=$(wc -l < "$SELECTION" | tr -d ' ')
base=$(rows | awk -F'\t' '$2=="base"' | wc -l | tr -d ' ')
whiptail --title "Ready to install" --yesno \
"Base: $base groups (always)
Selected extras: $n

Selection saved to:
$SELECTION

Install now? This needs sudo and an internet connection." 15 70 || { echo "Cancelled. Run ./install.sh $SELECTION later."; exit 0; }

exec "$DIR/install.sh" "$SELECTION"
