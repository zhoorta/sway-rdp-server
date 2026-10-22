#!/bin/bash
# usage: sample2.sh LABEL [SECONDS]; one run only (lock), CPU by process from /proc deltas, exact loopback bytes
label=$1; secs=${2:-60}; out=~/spike/measure/$label.csv
exec 9>~/spike/measure/.lock; flock -n 9 || { echo "another sampler is running"; exit 1; }
snap() { for p in $(ps -eo pid=,user= | awk '$2~/^rdptest/||$2=="sway-rdp"{print $1}'); do
  n=$(cat /proc/$p/comm 2>/dev/null); s=$(sed 's/^[0-9]* ([^)]*) //' /proc/$p/stat 2>/dev/null | awk '{print $12+$13}'); echo "$p $n $s"; done; }
lo() { awk '/ lo:/{print $2+$10}' /proc/net/dev; }
snap > /tmp/snap0.$$; b0=$(lo); : > $out; rsum=0
for i in $(seq 1 $secs); do sleep 1; r=$(ps -eo user=,rss= | awk '$1~/^rdptest/||$1=="sway-rdp"{x+=$2}END{printf "%.0f",x/1024}'); echo "$i,$r,$(lo)" >> $out; done
b1=$(lo); snap > /tmp/snap1.$$
echo "$label: ${secs}s, loopback $(( (b1-b0)/1024 )) KB = $(( (b1-b0)/1024/secs )) KB/s, rss avg $(awk -F, '{s+=$2}END{printf "%.0f",s/NR}' $out) MB"
join -j1 <(sort /tmp/snap0.$$) <(sort /tmp/snap1.$$) 2>/dev/null | awk -v t=$secs '{d=$5-$3; if(d>0) c[$2]+=d}END{for(k in c) printf "  %-18s %.1f%% of one core\n",k,c[k]/t}' | sort -k2 -rn | head -8
total=$(join -j1 <(sort /tmp/snap0.$$) <(sort /tmp/snap1.$$) | awk '{d=$5-$3;if(d>0)s+=d}END{print s}'); echo "  total ${total} ticks -> $(awk -v s=$total -v t=$secs 'BEGIN{printf "%.1f",s/t}')% of one core"
rm -f /tmp/snap0.$$ /tmp/snap1.$$
