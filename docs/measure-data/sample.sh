#!/bin/bash
# usage: sample.sh LABEL [SECONDS]; CPU from /proc utime+stime deltas (ticks=100/s), RSS from ps, loopback bytes exact
label=$1; secs=${2:-60}; mkdir -p ~/spike/measure; out=~/spike/measure/$label.csv
echo "t,cpu_pct,rss_mb,lo_bytes" > $out
ticks() { for p in $(ps -eo pid=,user= | awk '$2=="rdptest"||$2=="sway-rdp"{print $1}'); do cat /proc/$p/stat 2>/dev/null; done | sed 's/^[0-9]* ([^)]*) //' | awk '{s+=$12+$13}END{print s+0}'; }
lo() { awk '/ lo:/{print $2+$10}' /proc/net/dev; }
prev=$(ticks)
for i in $(seq 1 $secs); do
  sleep 1
  cur=$(ticks); rss=$(ps -eo user=,rss= | awk '$1=="rdptest"||$1=="sway-rdp"{r+=$2}END{printf "%.0f",r/1024}')
  d=$((cur-prev)); [ $d -lt 0 ] && d=0; prev=$cur
  echo "$i,$d,$rss,$(lo)" >> $out
done
awk -F, 'NR>1{n++;c+=$2;r+=$3;if($2>m)m=$2;if(NR==2)b0=$4;b1=$4}END{printf "%s: cpu avg %.1f%% max %d%% (of one core), rss avg %.0f MB, loopback total %.0f KB over %d s (%.1f KB/s)\n","'$label'",c/n,m,r/n,(b1-b0)/1024,n,(b1-b0)/n/1024}' $out
