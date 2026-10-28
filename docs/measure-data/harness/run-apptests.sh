#!/bin/bash
# run-apptests.sh : the app tests with the GPU on, then off. Output on stdout.
SERVER=${SERVER:?set SERVER to the machine under test}; CLIENT=${CLIENT:?set CLIENT to the machine that runs the RDP clients}
K=${SSH_KEY:-$HOME/.ssh/id_ed25519}
KH=${KNOWN_HOSTS:-$HOME/.ssh/known_hosts}
T() { ssh -i $K -o IdentitiesOnly=yes -o ServerAliveInterval=15 root@$SERVER "$@"; }
C() { ssh -i $K -o IdentitiesOnly=yes -o UserKnownHostsFile=$KH -o ServerAliveInterval=15 root@$CLIENT "$@"; }
for mode in on off; do
  T "sed -i '/^WAYDESK_GPU=/d' /etc/default/waydesk; [ $mode = off ] && echo WAYDESK_GPU=off >> /etc/default/waydesk; waydesk-admin users remove lt0 --delete-home >/dev/null 2>&1; true"
  J=$(T "waydesk-admin users add lt0 --json"); pw=$(printf '%s' "$J" | python3 -c 'import json,sys;print(json.load(sys.stdin)["password"])')
  C "SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy setsid nohup sdl-freerdp3 /v:$SERVER:3389 /u:lt0 '/p:$pw' /cert:ignore /size:1680x1050 /gfx:AVC420 > /root/client-apptest.log 2>&1 < /dev/null &"
  for t in $(seq 1 40); do T "waydesk-admin sessions list" | awk '{print $1}' | grep -qx lt0 && break; sleep 3; done
  sleep 30
  T "/root/load/apptest.sh lt0 GPU-$mode"
  C "pkill -f '[s]dl-freerdp3 /v:$SERVER'; true"; T "waydesk-admin sessions end lt0 >/dev/null 2>&1; sleep 3; waydesk-admin users remove lt0 --delete-home >/dev/null 2>&1; rm -rf /tmp/cr-lt0; true"
done
T "sed -i '/^WAYDESK_GPU=/d' /etc/default/waydesk; grep -c WAYDESK_GPU /etc/default/waydesk || true"
echo finished
