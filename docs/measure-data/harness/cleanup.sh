#!/bin/bash
# cleanup.sh : stop every load-test client and workload and remove the lt* accounts
pkill -f '[s]dl-freerdp3 /v:127.0.0.1 /u:lt'; pkill -f '[/]srv/load/work.sh'; sleep 3
for u in $(waydesk-admin users list 2>/dev/null | awk '{print $1}' | grep -E '^lt[0-9]+$'); do waydesk-admin users remove $u --delete-home >/dev/null 2>&1; done
waydesk-admin users list 2>/dev/null | grep -c '^lt' || true
