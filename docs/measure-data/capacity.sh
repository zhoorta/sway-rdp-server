#!/bin/bash
# Capacity test: heavy accounts one at a time, each from a short-lived xfreerdp3 on this server (never through a tunnel).
# Heavy = Chromium with three pages, LibreOffice Calc with a 300,000 x 10 CSV, VS Code (Flatpak) on a 28 MB source tree. Left idle.
OUT=/root/capacity; mkdir -p $OUT; : > $OUT/results.tsv; : > $OUT/pw.txt; chmod 600 $OUT/pw.txt
MAX=${1:-25}
avail() { free -m | awk '/^Mem:/ {print $7}'; }
swapused() { free -m | awk '/^Swap:/ {print $3}'; }
printf 'n\taccount\tavailable_MB\tswap_used_MB\tload1\tcpu_idle_pct\tuser_MB\tnote\n' >> $OUT/results.tsv
echo -e "0\t(baseline)\t$(avail)\t$(swapused)\t$(cut -d' ' -f1 /proc/loadavg)\t-\t-\tbefore the test" >> $OUT/results.tsv
for i in $(seq 1 $MAX); do
  name=cap$i
  J=$(waydesk-admin users add $name --json) || { echo -e "$i\t$name\t-\t-\t-\t-\t-\tusers add failed" >> $OUT/results.tsv; break; }
  pw=$(printf '%s' "$J" | python3 -c 'import json,sys;print(json.load(sys.stdin)["password"])')
  uid=$(printf '%s' "$J" | python3 -c 'import json,sys;print(json.load(sys.stdin)["uid"])')
  echo "$name $pw" >> $OUT/pw.txt
  d=$((100+i))
  Xvfb :$d -screen 0 1700x1100x24 >/dev/null 2>&1 & xv=$!
  sleep 2
  DISPLAY=:$d xfreerdp3 /v:127.0.0.1 /u:$name /p:"$pw" /size:1680x1050 /cert:ignore > $OUT/client-$i.log 2>&1 & cl=$!
  ok=0
  for t in $(seq 1 60); do
    waydesk-admin sessions list 2>/dev/null | awk '{print $1}' | grep -qx "$name" && { ok=1; break; }
    kill -0 $cl 2>/dev/null || break
    sleep 2
  done
  if [ $ok = 0 ]; then
    reason=$(grep -o -E "ERRINFO_[A-Z_]+|OUT_OF_MEMORY|REFUSED[^\"]*|not enough[^\"]*" $OUT/client-$i.log | head -1)
    echo -e "$i\t$name\t$(avail)\t$(swapused)\t$(cut -d' ' -f1 /proc/loadavg)\t-\t-\tREFUSED or failed: ${reason:-see client-$i.log}" >> $OUT/results.tsv
    kill $cl $xv 2>/dev/null; break
  fi
  sleep 25   # the shell comes up
  sock=$(ls /run/user/$uid/sway-ipc.*.sock 2>/dev/null | head -1)
  U="runuser -u $name -- env XDG_RUNTIME_DIR=/run/user/$uid WAYLAND_DISPLAY=wayland-1 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$uid/bus SWAYSOCK=$sock PATH=/usr/lib/waydesk/bin:/usr/bin:/bin HOME=/home/$name"
  $U sh -c 'cp /srv/capacity/data.csv ~/data.csv && cp -r /srv/capacity/tree ~/src'
  $U swaymsg exec "chromium --ozone-platform=wayland https://en.wikipedia.org/wiki/Linux https://www.bbc.com/news https://www.debian.org" >/dev/null
  sleep 8
  $U swaymsg exec "soffice --calc /home/$name/data.csv" >/dev/null
  sleep 8
  $U swaymsg exec "flatpak run com.visualstudio.code /home/$name/src" >/dev/null
  sleep 75
  um=$(waydesk-admin status 2>/dev/null | awk -v n="$name" '$1==n {print $3 $4}')
  idle=$(top -bn1 | awk '/Cpu\(s\)/ {for(i=1;i<=NF;i++) if($i=="id,"||$i=="id") print $(i-1)}' | head -1)
  echo -e "$i\t$name\t$(avail)\t$(swapused)\t$(cut -d' ' -f1 /proc/loadavg)\t${idle:--}\t${um:--}\tok" >> $OUT/results.tsv
  kill $cl $xv 2>/dev/null
  wait $cl $xv 2>/dev/null
done
echo "done $(date +%T)" >> $OUT/results.tsv
