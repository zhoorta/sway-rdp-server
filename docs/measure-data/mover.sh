#!/bin/bash
end=$((SECONDS+66)); i=0
while [ $SECONDS -lt $end ]; do
  x=$(( 500 + ($(( (i*40) % 400 )) ) )); y=$(( 250 + ( (i*25) % 250 ) ))
  sudo -u rdptest env XDG_RUNTIME_DIR=/run/user/1003 SWAYSOCK=/run/user/1003/sway-ipc.1003.54289.sock swaymsg "[title=\"drag-target\"] move position $x $y" >/dev/null 2>&1
  i=$((i+1)); sleep 0.03
done
echo moves=$i
