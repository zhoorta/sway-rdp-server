#!/bin/bash
# run-fallback.sh : what an NVIDIA-only box looks like to our encoder: GPU rendering and zero-copy capture on, but no usable VAAPI driver.
# One user on a full-screen WebGL page; the session must come up, the client must stay connected, frames must flow, and the software encoder must take over.
SERVER=${SERVER:?set SERVER to the machine under test}; CLIENT=${CLIENT:?set CLIENT to the machine that runs the RDP clients}
K=${SSH_KEY:-$HOME/.ssh/id_ed25519}
KH=${KNOWN_HOSTS:-$HOME/.ssh/known_hosts}
T() { ssh -i $K -o IdentitiesOnly=yes -o ServerAliveInterval=15 root@$SERVER "$@"; }
C() { ssh -i $K -o IdentitiesOnly=yes -o UserKnownHostsFile=$KH -o ServerAliveInterval=15 root@$CLIENT "$@"; }
for mode in normal novaapi; do
  echo "=== $mode"
  T "sed -i '/^WAYDESK_GPU=/d;/^LIBVA_DRIVER_NAME=/d;/^WAYDESK_LOG=/d' /etc/default/waydesk; echo WAYDESK_LOG=waydesk_server=debug,info >> /etc/default/waydesk; [ $mode = novaapi ] && echo LIBVA_DRIVER_NAME=does_not_exist >> /etc/default/waydesk; /root/load/cleanup.sh >/dev/null 2>&1; true"
  J=$(T "waydesk-admin users add lt1 --json"); pw=$(printf '%s' "$J" | python3 -c 'import json,sys;print(json.load(sys.stdin)["password"])'); uid=$(printf '%s' "$J" | python3 -c 'import json,sys;print(json.load(sys.stdin)["uid"])')
  S=$(date +%s)
  C "SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy setsid nohup sdl-freerdp3 /v:$SERVER:3389 /u:lt1 '/p:$pw' /cert:ignore /size:1680x1050 /gfx:AVC420 > /root/client-fallback.log 2>&1 < /dev/null &"
  for t in $(seq 1 40); do T "waydesk-admin sessions list" | awk '{print $1}' | grep -qx lt1 && break; sleep 3; done
  sleep 20
  T "sock=\$(ls /run/user/$uid/sway-ipc.*.sock | head -1); setsid nohup runuser -u lt1 -- env XDG_RUNTIME_DIR=/run/user/$uid WAYLAND_DISPLAY=wayland-1 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$uid/bus SWAYSOCK=\$sock PATH=/usr/lib/waydesk/bin:/usr/bin:/bin HOME=/home/lt1 USER=lt1 swaymsg exec 'chromium --user-data-dir=/tmp/cr-lt1 --ozone-platform=wayland --no-first-run --password-store=basic --kiosk file:///srv/load/gl.html' > /dev/null 2>&1 < /dev/null &"
  sleep 60
  echo "client still connected: $(C "pgrep -x sdl-freerdp3 >/dev/null && echo yes || echo NO")"
  echo "webgl fps in the session: $(T /root/load/fps.sh)"
  echo "--- encoder lines from the worker log:"
  T "journalctl --since '@$S' --no-pager | sed 's/\x1b\[[0-9;]*m//g' | grep -E 'encoder|VAAPI|falling back|software|sway renders|ERROR waydesk|WARN waydesk' | sed -E 's/.*waydesk_server[^ ]* //' | cut -c1-170 | sort | uniq -c | head -8"
  echo "--- frames acknowledged by the client in the last minute: $(T "journalctl --since '@$S' --no-pager | grep -c 'client acknowledged frame'")"
  echo "--- load: $(T "python3 /srv/load/sample.py 15" | python3 -c 'import json,sys;r=json.load(sys.stdin);print("cpu_busy",r["cpu_busy_pct"],"waydesk workers",r.get("cpu_waydesk"),"gpu_render",r.get("gpu_Render"),"gpu_video",r.get("gpu_Video"))')"
  C "pkill -f '[s]dl-freerdp3 /v:$SERVER'; true"; T "/root/load/cleanup.sh >/dev/null 2>&1; rm -rf /tmp/cr-lt1"
done
T "sed -i '/^LIBVA_DRIVER_NAME=/d;/^WAYDESK_LOG=/d' /etc/default/waydesk"
echo finished
