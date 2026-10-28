#!/bin/bash
# cleanup-remote.sh : stop the clients on the client machine, then the workloads and the lt* accounts on the machine under test.
SERVER=${SERVER:?set SERVER to the machine under test}; CLIENT=${CLIENT:?set CLIENT to the machine that runs the RDP clients}
K=${SSH_KEY:-$HOME/.ssh/id_ed25519}; KH=${KNOWN_HOSTS:-$HOME/.ssh/known_hosts}
ssh -i $K -o IdentitiesOnly=yes -o UserKnownHostsFile=$KH root@$CLIENT "pkill -f '[s]dl-freerdp3 /v:$SERVER'; true"
ssh -i $K -o IdentitiesOnly=yes root@$SERVER '/root/load/cleanup.sh'
