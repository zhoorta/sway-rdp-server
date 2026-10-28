#!/bin/bash
# ramp-remote.sh MAX LABEL : like ramp.sh, but the RDP clients run on a separate VPS ($CLIENT) and this loop runs on the Mac, driving both over SSH.
# One active user at a time (account + persistent client on the VPS + the workload inside the session; every fourth also loops a video), measured after each.
# Stops at MAX, a refusal, 97 % CPU on the server, or under 1.5 GB of memory there. Output: out-LABEL.tsv here.
MAX=${1:-24}; LABEL=${2:-remote}
SERVER=${SERVER:?set SERVER to the machine under test}; CLIENT=${CLIENT:?set CLIENT to the machine that runs the RDP clients}
K=${SSH_KEY:-$HOME/.ssh/id_ed25519}
KH=${KNOWN_HOSTS:-$HOME/.ssh/known_hosts}
T() { ssh -i $K -o IdentitiesOnly=yes -o ServerAliveInterval=15 root@$SERVER "$@"; }
C() { ssh -i $K -o IdentitiesOnly=yes -o UserKnownHostsFile=$KH -o ServerAliveInterval=15 root@$CLIENT "$@"; }
OUT=out-$LABEL.tsv
printf 'n\taccount\tprofile\tavail_MB\tswap_MB\tload1\tcpu_busy_pct\tcpu_waydesk\tcpu_sway\tcpu_chromium\tcpu_shell\tcpu_foot\tcpu_sshd\tcpu_harness\tcpu_other\tcpu_CLIENTS\tserver_side_pct\tgpu_Render\tgpu_Video\tgpu_VideoEnhance\tgpu_Blitter\tgpu_MHz\tpkg_W\terrors\tprobe_ms\tclient_vps_cpu_pct\tnote\n' > $OUT
for i in $(seq 1 $MAX); do
  name=lt$i; prof=worker; snd=''; [ $((i % 4)) = 0 ] && prof=video; case $i in 3|7|11) prof=videoaudio; snd=/sound;; esac; gimp=0; [ $((i % 4)) = 2 ] && gimp=1
  J=$(T "waydesk-admin users add $name --json") || { echo -e "$i\t$name\t$prof\tusers add failed" >> $OUT; break; }
  pw=$(printf '%s' "$J" | python3 -c 'import json,sys;print(json.load(sys.stdin)["password"])')
  uid=$(printf '%s' "$J" | python3 -c 'import json,sys;print(json.load(sys.stdin)["uid"])')
  C "SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy setsid nohup sdl-freerdp3 /v:$SERVER:3389 /u:$name '/p:$pw' /cert:ignore /size:1680x1050 /gfx:AVC420 $snd > /root/client-$LABEL-$i.log 2>&1 < /dev/null &"
  ok=0
  for t in $(seq 1 40); do
    T "waydesk-admin sessions list 2>/dev/null" | awk '{print $1}' | grep -qx "$name" && { ok=1; break; }
    sleep 3
  done
  if [ $ok = 0 ]; then echo -e "$i\t$name\t$prof\t$(T "free -m | awk '/^Mem:/ {print \$7}'")\tREFUSED or failed (client log on the VPS: client-$LABEL-$i.log)" >> $OUT; break; fi
  sleep 25
  T "sock=\$(ls /run/user/$uid/sway-ipc.*.sock 2>/dev/null | head -1); setsid nohup runuser -u $name -- env XDG_RUNTIME_DIR=/run/user/$uid WAYLAND_DISPLAY=wayland-1 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$uid/bus SWAYSOCK=\$sock PATH=/usr/lib/waydesk/bin:/usr/bin:/bin HOME=/home/$name USER=$name HEAVY=1 GIMP=$gimp /srv/load/work.sh $prof > /root/load/work-$LABEL-$i.log 2>&1 < /dev/null &"
  sleep 100
  S=$(date +%s)
  ( C "python3 -c \"
import time
def r():
    f=list(map(int,open('/proc/stat').readline().split()[1:])); return sum(f),f[3]+f[4]
a=r(); time.sleep(60); b=r(); print(round(100*(1-(b[1]-a[1])/(b[0]-a[0])),1))\"" > .clientcpu.$$ ) &
  row=$(T "python3 /srv/load/sample.py 60"); wait; ccpu=$(cat .clientcpu.$$); rm -f .clientcpu.$$
  extra=$(T "sock=\$(ls /run/user/$uid/sway-ipc.*.sock | head -1); t0=\$(date +%s%N); runuser -u $name -- env XDG_RUNTIME_DIR=/run/user/$uid SWAYSOCK=\$sock swaymsg -t get_version >/dev/null 2>&1; p=\$(( (\$(date +%s%N)-t0)/1000000 )); e=\$(journalctl --since '@$S' --no-pager 2>/dev/null | grep -cE 'ERROR waydesk|sending a frame failed|panicked'); a=\$(free -m | awk '/^Mem:/ {print \$7}'); s=\$(free -m | awk '/^Swap:/ {print \$3}'); l=\$(cut -d' ' -f1 /proc/loadavg); echo \$p \$e \$a \$s \$l")
  python3 - "$i" "$name" "$prof" "$row" "$extra" "$ccpu" >> $OUT <<'PY'
import sys,json
i,n,p,row,extra,cc=sys.argv[1:]; pr,er,av,sw,ld=extra.split(); r=json.loads(row); g=lambda k: r.get(k,'-')
ss=round(r.get('cpu_busy_pct',0)-r.get('cpu_CLIENTS',0)-r.get('cpu_harness',0),1)
print('\t'.join(map(str,[i,n,p,av,sw,ld,g('cpu_busy_pct'),g('cpu_waydesk'),g('cpu_sway'),g('cpu_chromium'),g('cpu_shell'),g('cpu_foot'),g('cpu_sshd'),g('cpu_harness'),g('cpu_other'),g('cpu_CLIENTS'),ss,g('gpu_Render'),g('gpu_Video'),g('gpu_VideoEnhance'),g('gpu_Blitter'),g('gpu_MHz'),g('pkg_W'),er,pr,cc,'ok'])))
PY
  tail -1 $OUT
  busy=$(tail -1 $OUT | cut -f7); av=$(tail -1 $OUT | cut -f4)
  [ "${av:-99999}" -lt 1500 ] && { echo "stop: memory" | tee -a $OUT; break; }
  python3 -c "import sys; sys.exit(0 if float('${busy:-0}')<97 else 1)" || { echo "stop: cpu" | tee -a $OUT; break; }
done
echo "done $(date +%T)" >> $OUT
