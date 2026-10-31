#!/bin/bash
# ramp-desktops.sh MAX LABEL DESKTOP [PROFILE] : like ramp-remote.sh, for comparing desktops on the same machine.
#   DESKTOP: sway-minimal    sway with the package's minimal configuration (account's ~/.config/waydesk/config = sway-minimal.conf)
#            waydesk-desktop sway with the Waydesk desktop (the system configuration of the waydesk-desktop package; needs it installed)
#            omarchy         Hyprland with the configuration Omarchy puts in /etc/skel (needs omarchy-settings and a GPU)
#            hyprland        Hyprland with the account's own configuration (no skel change; for ML4W and the like, prepare the home yourself)
#   PROFILE: mixed (default: as ramp-remote.sh: browser page that scrolls, typing terminal, every 4th user a 720p video loop, 3, 7, 11 with sound)
#            video  (every user loops a 720p video: the worst case for the encoder; phase 2, run it after a mixed run, with N chosen from it)
# One active user at a time, measured after each. The criteria are stated BEFORE the run and recorded in the header of the output:
#   STOP_AVAIL_MB  (1500)  memory: available memory under this = stop "memory"
#   STOP_CPU       (90)    CPU: busy share of the server's threads over the 60 s sample, minus the harness = stop "cpu"
#   STOP_FPS       (0)     encoder: median frames per second of the sessions under this = stop "encoder" (set 12 for PROFILE=video; mixed screens are mostly still)
#   STOP_CLIENT_CPU (60)   validity: the client machine busier than this = stop "client saturated" (the step does not count)
#   plus a refusal by the admission rule = stop "refused". Each criterion is reported separately; capacity = the last step before the FIRST stop.
# Output: out-LABEL.tsv, raw-LABEL/step-N.json (waydesk-admin status --json after each step), raw-LABEL/system.txt. Run from this folder; needs SERVER and CLIENT (see README.md).
MAX=${1:-24}; LABEL=${2:?label}; DESKTOP=${3:?desktop}; PROFILE=${4:-mixed}
STOP_AVAIL_MB=${STOP_AVAIL_MB:-1500}; STOP_CPU=${STOP_CPU:-90}; STOP_FPS=${STOP_FPS:-0}; STOP_CLIENT_CPU=${STOP_CLIENT_CPU:-60}
SERVER=${SERVER:?set SERVER to the machine under test}; CLIENT=${CLIENT:?set CLIENT to the machine that runs the RDP clients}
K=${SSH_KEY:-$HOME/.ssh/id_ed25519}
KH=${KNOWN_HOSTS:-$HOME/.ssh/known_hosts}
SKH=${SERVER_KNOWN_HOSTS:-$KH}
T() { ssh -i $K -o IdentitiesOnly=yes -o UserKnownHostsFile=$SKH -o ServerAliveInterval=15 root@$SERVER "$@"; }
C() { ssh -i $K -o IdentitiesOnly=yes -o UserKnownHostsFile=$KH -o ServerAliveInterval=15 root@$CLIENT "$@"; }
case $DESKTOP in
  sway-minimal|waydesk-desktop) COMP=sway; SETDESK=sway;;
  omarchy|hyprland) COMP=hyprland; SETDESK=hyprland;;
  *) echo "unknown DESKTOP $DESKTOP" >&2; exit 2;;
esac
OUT=out-$LABEL.tsv; RAW=raw-$LABEL; mkdir -p $RAW
T 'uname -a; echo; cat /etc/os-release | head -3; echo; lscpu | grep -E "Model name|^CPU\(s\)"; free -m; echo; waydesk-admin status 2>&1 | head -30; echo; cat /etc/default/waydesk 2>/dev/null; pacman -Q waydesk sway hyprland omarchy 2>&1 | head' > $RAW/system.txt 2>&1
{ printf '# ramp-desktops.sh label=%s desktop=%s profile=%s max=%s date=%s\n' "$LABEL" "$DESKTOP" "$PROFILE" "$MAX" "$(date -u +%FT%TZ)"
  printf '# criteria: STOP_AVAIL_MB=%s STOP_CPU=%s STOP_FPS=%s STOP_CLIENT_CPU=%s (plus admission refusal)\n' "$STOP_AVAIL_MB" "$STOP_CPU" "$STOP_FPS" "$STOP_CLIENT_CPU"
  printf 'n\taccount\tprofile\tavail_MB\tswap_MB\tuser_slice_MB\tload1\tcpu_busy_pct\tcpu_waydesk\tcpu_sway\tcpu_chromium\tcpu_shell\tcpu_foot\tcpu_sshd\tcpu_harness\tcpu_other\tcpu_CLIENTS\tserver_side_pct\tgpu_Render\tgpu_Video\tgpu_VideoEnhance\tgpu_Blitter\tgpu_MHz\tpkg_W\terrors\tprobe_ms\tfps_min\tfps_med\tkbps_total\tsessions_current\tclient_vps_cpu_pct\tnote\n'; } > $OUT
stop() { echo -e "stop: $1" | tee -a $OUT; STOPPED=1; }
STOPPED=0
for i in $(seq 1 $MAX); do
  name=lt$i; prof=worker; snd=''; gimp=0
  if [ "$PROFILE" = video ]; then prof=video; else
    [ $((i % 4)) = 0 ] && prof=video; case $i in 3|7|11) prof=videoaudio; snd=/sound;; esac
  fi
  J=$(T "waydesk-admin users add $name --json") || { echo -e "$i\t$name\t$prof\tusers add failed" >> $OUT; break; }
  pw=$(printf '%s' "$J" | python3 -c 'import json,sys;print(json.load(sys.stdin)["password"])')
  uid=$(printf '%s' "$J" | python3 -c 'import json,sys;print(json.load(sys.stdin)["uid"])')
  T "waydesk-admin users set $name --desktop $SETDESK" >/dev/null
  [ "$DESKTOP" = sway-minimal ] && T "install -d -o $name -g $name /home/$name/.config/waydesk && install -o $name -g $name /usr/share/waydesk/sway-minimal.conf /home/$name/.config/waydesk/config"
  C "SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy setsid nohup sdl-freerdp3 /v:$SERVER:3389 /u:$name '/p:$pw' /cert:ignore /size:1680x1050 /gfx:AVC420 $snd > /root/client-$LABEL-$i.log 2>&1 < /dev/null &"
  ok=0
  for t in $(seq 1 40); do
    T "waydesk-admin sessions list 2>/dev/null" | awk '{print $1}' | grep -qx "$name" && { ok=1; break; }
    sleep 3
  done
  if [ $ok = 0 ]; then echo -e "$i\t$name\t$prof\t$(T "free -m | awk '/^Mem:/ {print \$7}'")\tREFUSED or failed (client log on the VPS: client-$LABEL-$i.log)" >> $OUT; stop refused; break; fi
  sleep 25
  # the session's sockets: sway has its own IPC socket, Hyprland an instance directory; the Wayland display is the same search for both
  T "wl=\$(ls /run/user/$uid | grep -E '^wayland-[0-9]+\$' | head -1); sock=\$(ls /run/user/$uid/sway-ipc.*.sock 2>/dev/null | head -1); sig=\$(ls /run/user/$uid/hypr 2>/dev/null | head -1); setsid nohup runuser -u $name -- env XDG_RUNTIME_DIR=/run/user/$uid WAYLAND_DISPLAY=\${wl:-wayland-1} DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$uid/bus SWAYSOCK=\$sock HYPRLAND_INSTANCE_SIGNATURE=\$sig COMPOSITOR=$COMP PATH=/usr/lib/waydesk/bin:/usr/bin:/bin HOME=/home/$name USER=$name HEAVY=0 GIMP=$gimp /srv/load/work.sh $prof > /root/load/work-$LABEL-$i.log 2>&1 < /dev/null &"
  sleep 100
  S=$(date +%s)
  ( C "python3 -c \"
import time
def r():
    f=list(map(int,open('/proc/stat').readline().split()[1:])); return sum(f),f[3]+f[4]
a=r(); time.sleep(60); b=r(); print(round(100*(1-(b[1]-a[1])/(b[0]-a[0])),1))\"" > .clientcpu.$$ ) &
  row=$(T "python3 /srv/load/sample.py 60"); wait; ccpu=$(cat .clientcpu.$$); rm -f .clientcpu.$$
  T "waydesk-admin status --json" > $RAW/step-$i.json 2>/dev/null
  if [ "$COMP" = sway ]; then PROBE="sock=\$(ls /run/user/$uid/sway-ipc.*.sock | head -1); runuser -u $name -- env XDG_RUNTIME_DIR=/run/user/$uid SWAYSOCK=\$sock swaymsg -t get_version >/dev/null 2>&1"
  else PROBE="sig=\$(ls /run/user/$uid/hypr | head -1); runuser -u $name -- env XDG_RUNTIME_DIR=/run/user/$uid HYPRLAND_INSTANCE_SIGNATURE=\$sig hyprctl version >/dev/null 2>&1"; fi
  extra=$(T "t0=\$(date +%s%N); $PROBE; p=\$(( (\$(date +%s%N)-t0)/1000000 )); e=\$(journalctl --since '@$S' --no-pager 2>/dev/null | grep -cE 'ERROR waydesk|sending a frame failed|panicked'); a=\$(free -m | awk '/^Mem:/ {print \$7}'); s=\$(free -m | awk '/^Swap:/ {print \$3}'); l=\$(cut -d' ' -f1 /proc/loadavg); u=\$(( \$(cat /sys/fs/cgroup/user.slice/memory.current) / 1048576 )); echo \$p \$e \$a \$s \$l \$u")
  python3 - "$i" "$name" "$prof" "$row" "$extra" "$ccpu" "$RAW/step-$i.json" >> $OUT <<'PY'
import sys,json,statistics
i,n,p,row,extra,cc,rawf=sys.argv[1:]; pr,er,av,sw,ld,us=extra.split(); r=json.loads(row); g=lambda k: r.get(k,'-')
ss=round(r.get('cpu_busy_pct',0)-r.get('cpu_CLIENTS',0)-r.get('cpu_harness',0),1)
fps=[]; kb=0; cur=0
try:
    for u in json.load(open(rawf)).get('users',[]):
        v=u.get('video')
        if v and v.get('current') and v.get('fps') is not None: fps.append(v['fps']); kb+=v.get('kbps') or 0; cur+=1
except Exception: pass
print('\t'.join(map(str,[i,n,p,av,sw,us,ld,g('cpu_busy_pct'),g('cpu_waydesk'),g('cpu_sway'),g('cpu_chromium'),g('cpu_shell'),g('cpu_foot'),g('cpu_sshd'),g('cpu_harness'),g('cpu_other'),g('cpu_CLIENTS'),ss,g('gpu_Render'),g('gpu_Video'),g('gpu_VideoEnhance'),g('gpu_Blitter'),g('gpu_MHz'),g('pkg_W'),er,pr,min(fps) if fps else '-',round(statistics.median(fps),1) if fps else '-',kb,cur,cc,'ok'])))
PY
  tail -1 $OUT
  av=$(tail -1 $OUT | cut -f4); ss=$(tail -1 $OUT | cut -f18); fm=$(tail -1 $OUT | cut -f28)
  [ "${av:-99999}" -lt "$STOP_AVAIL_MB" ] && stop "memory (available ${av} MB at step $i)"
  python3 -c "import sys; sys.exit(0 if float('${ss:-0}')<$STOP_CPU else 1)" || stop "cpu (server side ${ss} % at step $i)"
  python3 -c "import sys; sys.exit(0 if float('${ccpu:-0}')<$STOP_CLIENT_CPU else 1)" || stop "client saturated (${ccpu} % at step $i: this step and later ones do not count)"
  [ "$STOP_FPS" != 0 ] && [ "$fm" != "-" ] && { python3 -c "import sys; sys.exit(0 if float('$fm')>=$STOP_FPS else 1)" || stop "encoder (median ${fm} fps at step $i)"; }
  [ $STOPPED = 1 ] && break
done
echo "done $(date +%T)" >> $OUT
