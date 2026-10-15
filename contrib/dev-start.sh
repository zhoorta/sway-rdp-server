#!/bin/sh
# Development helper: run inside a VM with a sway session. Starts sway-rdp-server; it creates a headless sway output per connection.
# --kb-layout is only the fallback for clients that announce no keyboard layout.
export XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-1 SWAY_RDP_LOG=${SWAY_RDP_LOG:-info}
export SWAYSOCK=$(ls /run/user/1000/sway-ipc.*.sock | head -1)
pkill sway-rdp-server 2>/dev/null; pkill hypr-rdp 2>/dev/null; sleep 1
# Sessions left behind by a server that was killed mid-connection: their outputs and their shell processes.
pkill -f '[b]urind.py --output' 2>/dev/null; pkill -f '[p]anel.py .* --port' 2>/dev/null
for o in $(swaymsg -t get_outputs -r | python3 -c 'import json,sys; print(*[o["name"] for o in json.load(sys.stdin) if o["name"].startswith("HEADLESS")])'); do
  swaymsg "[workspace=\"Remote\"] kill" >/dev/null; swaymsg "[con_mark=\"^from:$o\$\"] kill" >/dev/null; swaymsg output "$o" unplug >/dev/null
done
# An empty workspace left behind by a session goes away once it is not visible.
swaymsg workspace 1 >/dev/null
# The boot-time pin did not stick on a headless start (came up 1920x1080 @1.25).
swaymsg output Virtual-1 mode 1680x1050 scale 1 >/dev/null; sleep 1
cd ~/spike/sway-rdp-server || exit 1
nohup ./target/release/sway-rdp-server --bind 127.0.0.1:3389 --cert ~/spike/tls/cert.pem --key ~/spike/tls/key.pem \
  --kb-layout pt --credentials ~/spike/rdp.cred "$@" > ~/spike/sway-rdp-server.log 2>&1 &
sleep 3
ss -ltn | grep -q 3389 && echo "sway-rdp-server listening on 127.0.0.1:3389" || { echo "failed:"; tail -5 ~/spike/sway-rdp-server.log; exit 1; }
