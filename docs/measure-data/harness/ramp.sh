#!/bin/bash
# ramp.sh MAX LABEL : add one active user at a time (a persistent RDP client on this server + a workload inside the session), measure after each.
# Every fourth user also loops a 720p video. Stops at MAX, a refusal, 97 % CPU or under 1.5 GB of memory. Output: /root/load/out-LABEL.tsv
MAX=${1:-16}; LABEL=${2:-gpu}; OUT=/root/load/out-$LABEL.tsv; mkdir -p /root/load; : > /root/load/pw-$LABEL.txt
avail() { free -m | awk '/^Mem:/ {print $7}'; }
swapused() { free -m | awk '/^Swap:/ {print $3}'; }
printf 'n\taccount\tprofile\tavail_MB\tswap_MB\tload1\tcpu_busy_pct\tcpu_waydesk\tcpu_sway\tcpu_chromium\tcpu_shell\tcpu_foot\tcpu_sshd\tcpu_harness\tcpu_other\tcpu_CLIENTS\tserver_side_pct\tgpu_Render\tgpu_Video\tgpu_VideoEnhance\tgpu_Blitter\tgpu_MHz\tpkg_W\terrors\tprobe_ms\tnote\n' > $OUT
for i in $(seq 1 $MAX); do
  name=lt$i; prof=worker; [ $((i % 4)) = 0 ] && prof=video
  J=$(waydesk-admin users add $name --json) || { echo -e "$i\t$name\t$prof\tusers add failed" >> $OUT; break; }
  pw=$(printf '%s' "$J" | python3 -c 'import json,sys;print(json.load(sys.stdin)["password"])')
  uid=$(printf '%s' "$J" | python3 -c 'import json,sys;print(json.load(sys.stdin)["uid"])')
  echo "$name $pw" >> /root/load/pw-$LABEL.txt
  SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy setsid sdl-freerdp3 /v:127.0.0.1 /u:$name "/p:$pw" /cert:ignore /size:1680x1050 /gfx:AVC420 > /root/load/client-$LABEL-$i.log 2>&1 &
  cl=$!; ok=0
  for t in $(seq 1 60); do
    waydesk-admin sessions list 2>/dev/null | awk '{print $1}' | grep -qx "$name" && { ok=1; break; }
    kill -0 $cl 2>/dev/null || break; sleep 2
  done
  if [ $ok = 0 ]; then echo -e "$i\t$name\t$prof\t$(avail)\t$(swapused)\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\tREFUSED or failed (see client-$LABEL-$i.log)" >> $OUT; break; fi
  sleep 25
  sock=$(ls /run/user/$uid/sway-ipc.*.sock 2>/dev/null | head -1)
  ENVV="XDG_RUNTIME_DIR=/run/user/$uid WAYLAND_DISPLAY=wayland-1 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$uid/bus SWAYSOCK=$sock PATH=/usr/lib/waydesk/bin:/usr/bin:/bin HOME=/home/$name USER=$name"
  setsid nohup runuser -u $name -- env $ENVV /srv/load/work.sh $prof > /root/load/work-$LABEL-$i.log 2>&1 &
  sleep 45   # the workload starts and settles
  S=$(date +%s); row=$(python3 /srv/load/sample.py 60)
  t0=$(date +%s%N); runuser -u $name -- env $ENVV swaymsg -t get_version >/dev/null 2>&1; probe=$(( ($(date +%s%N) - t0) / 1000000 ))
  errs=$(journalctl --since "@$S" --no-pager 2>/dev/null | grep -cE "ERROR waydesk|sending a frame failed|panicked")
  python3 - "$i" "$name" "$prof" "$(avail)" "$(swapused)" "$(cut -d' ' -f1 /proc/loadavg)" "$row" "$errs" "$probe" >> $OUT <<'PY'
import sys,json
i,n,p,av,sw,ld,row,er,pr=sys.argv[1:]; r=json.loads(row); g=lambda k: r.get(k,'-')
ss=round(r.get('cpu_busy_pct',0)-r.get('cpu_CLIENTS',0)-r.get('cpu_harness',0),1)
print('\t'.join(map(str,[i,n,p,av,sw,ld,g('cpu_busy_pct'),g('cpu_waydesk'),g('cpu_sway'),g('cpu_chromium'),g('cpu_shell'),g('cpu_foot'),g('cpu_sshd'),g('cpu_harness'),g('cpu_other'),g('cpu_CLIENTS'),ss,g('gpu_Render'),g('gpu_Video'),g('gpu_VideoEnhance'),g('gpu_Blitter'),g('gpu_MHz'),g('pkg_W'),er,pr,'ok'])))
PY
  tail -1 $OUT
  busy=$(tail -1 $OUT | cut -f7); av=$(avail)
  if [ "$av" -lt 1500 ]; then echo "stop: memory" | tee -a $OUT; break; fi
  python3 -c "import sys; sys.exit(0 if float('${busy:-0}')<97 else 1)" || { echo "stop: cpu" | tee -a $OUT; break; }
done
echo "done $(date +%T)" >> $OUT
