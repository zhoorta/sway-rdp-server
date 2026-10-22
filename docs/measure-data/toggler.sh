#!/bin/sh
# toggles DMS popouts about once a second until killed (run as rdptest)
export XDG_RUNTIME_DIR=/run/user/1003 WAYLAND_DISPLAY=wayland-1 LANG=C.UTF-8
while :; do
  dms ipc call control-center toggle >/dev/null 2>&1; sleep 0.9
  dms ipc call dash toggle overview >/dev/null 2>&1; sleep 0.9
done
