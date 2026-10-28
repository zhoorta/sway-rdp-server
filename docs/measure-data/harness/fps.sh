#!/bin/bash
# fps.sh : the frame rate every lt* user's WebGL page shows in its own window title ("fps N")
for u in $(waydesk-admin sessions list | awk '/^lt/{print $1}'); do
  uid=$(id -u $u); sock=$(ls /run/user/$uid/sway-ipc.*.sock 2>/dev/null | head -1)
  t=$(runuser -u $u -- env XDG_RUNTIME_DIR=/run/user/$uid SWAYSOCK=$sock PATH=/usr/lib/waydesk/bin:/usr/bin:/bin swaymsg -t get_tree 2>/dev/null | grep -o '"fps [0-9]*' | tail -1 | grep -o '[0-9]*')
  echo -n "${t:-0} "
done; echo
