#!/bin/bash
# apptest.sh USER LABEL : (on the test server, USER has a running session) start each app, wait, report: alive, windows, error lines, GPU busy
U=$1; LABEL=$2; uid=$(id -u $U); sock=$(ls /run/user/$uid/sway-ipc.*.sock | head -1)
E="XDG_RUNTIME_DIR=/run/user/$uid WAYLAND_DISPLAY=wayland-1 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$uid/bus SWAYSOCK=$sock PATH=/usr/lib/waydesk/bin:/usr/bin:/bin HOME=/home/$U USER=$U"
R() { runuser -u $U -- env $E "$@"; }
wins() { R swaymsg -t get_tree | python3 -c "
import json,sys
def walk(n):
    if n.get('pid') and n.get('type') in ('con','floating_con'): print((n.get('app_id') or (n.get('window_properties') or {}).get('class') or '?')+' | '+(n.get('name') or '')[:50])
    for c in n.get('nodes',[])+n.get('floating_nodes',[]): walk(c)
walk(json.load(sys.stdin))" | sort; }
R sh -c 'cp -r /srv/capacity/tree ~/src 2>/dev/null; cp /srv/load/img.jpg ~/img.jpg 2>/dev/null; mkdir -p ~/proj'
echo "=== $LABEL: GL/Vulkan as the session sees them"
R sh -c 'glxinfo -B 2>/dev/null | grep -E "OpenGL renderer|OpenGL version" ; vulkaninfo --summary 2>/dev/null | grep -E "deviceName|apiVersion" | head -2' | sed 's/^/   /'
while IFS='|' read -r name pat cmd; do
  before=$(wins | wc -l)
  R swaymsg exec "sh -c 'exec $cmd > /home/$U/app-$name.log 2>&1'" >/dev/null; sleep 30
  alive=no; pgrep -u $U -f "$pat" >/dev/null && alive=yes
  new=$(( $(wins | wc -l) - before ))
  gpu=$(timeout -s INT 5 intel_gpu_top -J -s 1000 2>/dev/null | grep -A1 '"Render/3D"' | grep busy | sed -E 's/.*: "?([0-9.]+).*/\1/' | sort -n | tail -1)
  err=$(grep -iE "failed|error|cannot|can't|unable|no suitable|not supported|abort|panic" /home/$U/app-$name.log | head -2 | cut -c1-110 | tr '\n' ';')
  printf "%-9s alive=%-3s new_windows=%s gpu_render_peak=%s%%  %s\n" $name $alive $new "${gpu:-?}" "$err"
  R pkill -f "$pat" 2>/dev/null; sleep 4
done <<'APPS'
zed|dev.zed.Zed|flatpak run dev.zed.Zed ~/src
blender|bin/blender|blender
kdenlive|kdenlive|kdenlive
darktable|darktable|darktable ~/img.jpg
vkcube|vkcube|vkcube --wsi wayland
glxgears|glxgears|glxgears
APPS
