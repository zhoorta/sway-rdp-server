#!/bin/bash
# run-webgl.sh : WebGL smoothness. Phase 1: GPU on, users added one at a time up to 12, each on a full-screen WebGL page; Phase 2: GPU off, 1 and 2 users.
SERVER=${SERVER:?set SERVER to the machine under test}; CLIENT=${CLIENT:?set CLIENT to the machine that runs the RDP clients}
K=${SSH_KEY:-$HOME/.ssh/id_ed25519}
KH=${KNOWN_HOSTS:-$HOME/.ssh/known_hosts}
T() { ssh -i $K -o IdentitiesOnly=yes -o ServerAliveInterval=15 root@$SERVER "$@"; }
C() { ssh -i $K -o IdentitiesOnly=yes -o UserKnownHostsFile=$KH -o ServerAliveInterval=15 root@$CLIENT "$@"; }
OUT=out-webgl.tsv
printf 'phase\tusers\tfps_each\tfps_min\tfps_median\tfps_max\tcpu_busy_pct\tgpu_Render\tgpu_Video\tgpu_MHz\tpkg_W\n' > $OUT
phase() {  # phase LABEL MAXUSERS "COUNTS TO MEASURE AT"
  label=$1; max=$2; at=" $3 "
  for i in $(seq 1 $max); do
    name=lt$i
    J=$(T "waydesk-admin users add $name --json") || break
    pw=$(printf '%s' "$J" | python3 -c 'import json,sys;print(json.load(sys.stdin)["password"])')
    uid=$(printf '%s' "$J" | python3 -c 'import json,sys;print(json.load(sys.stdin)["uid"])')
    C "SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy setsid nohup sdl-freerdp3 /v:$SERVER:3389 /u:$name '/p:$pw' /cert:ignore /size:1680x1050 /gfx:AVC420 > /root/client-webgl-$i.log 2>&1 < /dev/null &"
    for t in $(seq 1 40); do T "waydesk-admin sessions list" | awk '{print $1}' | grep -qx "$name" && break; sleep 3; done
    sleep 20
    T "sock=\$(ls /run/user/$uid/sway-ipc.*.sock | head -1); setsid nohup runuser -u $name -- env XDG_RUNTIME_DIR=/run/user/$uid WAYLAND_DISPLAY=wayland-1 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$uid/bus SWAYSOCK=\$sock PATH=/usr/lib/waydesk/bin:/usr/bin:/bin HOME=/home/$name USER=$name swaymsg exec 'chromium --user-data-dir=/tmp/cr-$name --ozone-platform=wayland --no-first-run --password-store=basic --kiosk file:///srv/load/gl.html' > /dev/null 2>&1 < /dev/null &"
    sleep 35
    if [[ "$at" == *" $i "* ]]; then
      fps=$(T "/root/load/fps.sh"); row=$(T "python3 /srv/load/sample.py 20")
      python3 - "$label" "$i" "$fps" "$row" >> $OUT <<'PY'
import sys,json,statistics
l,n,fps,row=sys.argv[1:]; v=[int(x) for x in fps.split()] or [0]; r=json.loads(row); g=lambda k:r.get(k,'-')
print('\t'.join(map(str,[l,n,' '.join(map(str,v)),min(v),statistics.median(v),max(v),g('cpu_busy_pct'),g('gpu_Render'),g('gpu_Video'),g('gpu_MHz'),g('pkg_W')])))
PY
      tail -1 $OUT
    fi
  done
}
cleanup() { C "pkill -f '[s]dl-freerdp3 /v:$SERVER'; true"; T "/root/load/cleanup.sh; rm -rf /tmp/cr-lt*" >/dev/null 2>&1; }
T "sed -i '/^WAYDESK_GPU=/d' /etc/default/waydesk"; cleanup
phase gpu-on 12 "1 2 3 4 6 8 10 12"
cleanup
T "echo WAYDESK_GPU=off >> /etc/default/waydesk"
phase gpu-off 2 "1 2"
cleanup
T "sed -i '/^WAYDESK_GPU=/d' /etc/default/waydesk"
echo "done $(date +%T)" >> $OUT
