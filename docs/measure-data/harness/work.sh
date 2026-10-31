#!/bin/bash
# work.sh PROFILE : runs as the account, inside its session.
# PROFILE: worker (light: a document that scrolls, a terminal that types), video (+ a muted 720p loop), videoaudio (+ the loop with sound).
# HEAVY=1 adds LibreOffice Calc (300,000 rows), VS Code on a source tree and keystrokes into them (wtype); GIMP=1 adds GIMP on a 4000x3000 image (zoom in and out).
P=${1:-worker}
cd ~   # the driver starts this from /root, which the account cannot read: foot, soffice and the others fail to start from there
# COMPOSITOR=hyprland (a session that is not sway): the few swaymsg forms used below are translated: exec = hl.exec_cmd (the program inherits the session's environment, DISPLAY included), [app_id|class|title="X"] focus|kill = Lua through
# hyprctl eval (windows matched by a lower-case part of the class or title; verified on Hyprland 0.57: hl.get_windows() + hl.dsp.focus({ window = w })), -t get_tree = the client list.
wd_hl() { hyprctl eval "$1" >/dev/null 2>&1; }
# COMPOSITOR=cosmic: the session's sway only holds COSMIC as one fullscreen window (docs/COSMIC.md); the programs belong in COSMIC, whose display is the second one
# of the runtime directory (wayland-2). There is nothing to focus or close from the outside: a new window takes the focus, and the keys (wtype) go to it.
if [ "$COMPOSITOR" = cosmic ]; then
  for _ in $(seq 1 60); do [ -S "$XDG_RUNTIME_DIR/wayland-2" ] && break; sleep 1; done
  export WAYLAND_DISPLAY=wayland-2
fi
wd_sm() {
  if [ "$COMPOSITOR" = cosmic ]; then
    case "$1" in
      exec) setsid sh -c "$2" >/dev/null 2>&1 < /dev/null & ;;
      -t) echo ;;
    esac
    return
  fi
  [ "$COMPOSITOR" = hyprland ] || { swaymsg "$@"; return; }
  case "$1" in
    exec) wd_hl "hl.exec_cmd([==[$2]==])" ;;   # through the compositor, so the program gets its DISPLAY (Flatpak VS Code needs X11); a plain setsid from this shell has none
    -t) hyprctl clients -j ;;
    \[*)
      local f=class; case "$1" in \[title=*) f=title;; esac
      local key; key=$(printf '%s' "$1" | sed -E 's/^\[[a-z_]+="([^"]*)"\]$/\1/; s/\.\*$//' | tr 'A-Z' 'a-z')
      case "$2" in
        focus) wd_hl "for _, w in ipairs(hl.get_windows()) do if string.find(string.lower(w.$f or ''), '$key', 1, true) then hl.dispatch(hl.dsp.focus({ window = w })) break end end" ;;
        kill)  wd_hl "for _, w in ipairs(hl.get_windows()) do if string.find(string.lower(w.$f or ''), '$key', 1, true) then hl.dispatch(hl.dsp.window.close({ window = w })) end end" ;;
      esac ;;
  esac
}
SM=wd_sm
CH="chromium --ozone-platform=wayland --no-first-run --password-store=basic"
$SM exec "$CH --user-data-dir=/tmp/cr-$USER --new-window file:///srv/load/page.html" >/dev/null; sleep 6
$SM exec "foot -a work -T work /srv/load/term.sh" >/dev/null; sleep 3
[ "$P" = videoaudio ] && { $SM exec "$CH --user-data-dir=/tmp/cr-v-$USER --autoplay-policy=no-user-gesture-required --new-window file:///srv/load/video-audio.html" >/dev/null; sleep 6; }
[ "$P" = video ] && { $SM exec "$CH --user-data-dir=/tmp/cr-v-$USER --new-window file:///srv/load/video.html" >/dev/null; sleep 6; }
if [ "$HEAVY" = 1 ]; then
  cp /srv/capacity/data.ods ~/data.ods; cp -r /srv/capacity/tree ~/src
  mkdir -p ~/.var/app/com.visualstudio.code/config/Code/User
  echo '{"security.workspace.trust.enabled": false, "workbench.startupEditor": "none", "update.mode": "none", "telemetry.telemetryLevel": "off", "extensions.autoUpdate": false}' > ~/.var/app/com.visualstudio.code/config/Code/User/settings.json
  $SM exec "soffice --calc ~/data.ods" >/dev/null; sleep 15
  VSC="flatpak run"; [ "$COMPOSITOR" = cosmic ] && VSC="flatpak run --socket=wayland --env=ELECTRON_OZONE_PLATFORM_HINT=wayland"   # COSMIC has no DISPLAY for it to fall back on
  $SM exec "$VSC com.visualstudio.code ~/src ~/src/os.py" >/dev/null; sleep 15
  [ "$GIMP" = 1 ] && { cp /srv/load/img.jpg ~/img.jpg; $SM exec "gimp ~/img.jpg" >/dev/null; sleep 25; }
  dismiss() { for t in "Tip of the Day" "Welcome to GIMP"; do $SM -t get_tree | grep -q "$t" && { $SM "[title=\"$t.*\"] kill" >/dev/null 2>&1; sleep 1; }; done; }
  keys() { sleep 0.4; for _ in $(seq 1 $2); do wtype -k "$1"; sleep 1; done; }
  for _ in 1 2 3 4 5 6; do dismiss; sleep 8; done
  while true; do
    dismiss; sleep $((RANDOM%10+6))
    case $((RANDOM%3)) in
      0) $SM '[app_id="code"] focus' >/dev/null 2>&1; keys Page_Down 4; wtype "total = compute(value) + 1"; wtype -k Return ;;
      1) $SM '[class="LibreOffice.*"] focus' >/dev/null 2>&1; keys Page_Down 5; [ $((RANDOM%3)) = 0 ] && wtype -M ctrl -k Home -m ctrl ;;
      2) if [ "$GIMP" = 1 ]; then $SM '[app_id="gimp"] focus' >/dev/null 2>&1; sleep 0.4; wtype "+"; sleep 1; wtype "+"; sleep 1.5; wtype "-"; sleep 1; wtype "-"; else $SM '[app_id="work"] focus' >/dev/null 2>&1; fi ;;
    esac
  done
fi
while true; do
  sleep $((RANDOM%16+15)); $SM '[app_id="work"] focus' >/dev/null 2>&1
  sleep $((RANDOM%16+15)); $SM '[app_id="chromium"] focus' >/dev/null 2>&1
done
