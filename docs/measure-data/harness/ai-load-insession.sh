#!/bin/bash
# ai-load-insession.sh LABEL GPUMODE ["N N"] ["C C"] [AIU] [PROFILE] [DESKTOP] : like ai-load.sh, but the model is called FROM INSIDE the sessions: the first AIU sessions each run llm-bench.py as
# their own account (runuser -u ltK), C concurrent requests each, so the load comes from the desktops' accounts, `waydesk-admin status` shows it per account (AI column) and the test shows that
# a session can reach the local service. N must be at least AIU (default 3), C is per account (default "1 2": 3 and 6 requests in flight with NUM_PARALLEL=4, so the second window queues).
# Everything else is as in ai-load.sh below (that text follows).
# ai-load.sh LABEL GPUMODE ["N N N"] ["C C C"] [PROFILE] [DESKTOP] : desktops and a local model on the same server, measured together (plan: AI-LOAD-PLAN.md).
#   GPUMODE: igpu     every session on the integrated GPU (Intel); the model on the NVIDIA card        (expected: sessions do not notice the model)
#            rtx      every session on the NVIDIA card, the model on the same card                      (expected: encoder fine, render engine and memory shared)
#            balance  WAYDESK_GPU=balance for every account (set WAYDESK_GPU_RESERVE on the server first if the model's card should be left alone)
#            software every session in software (WAYDESK_GPU=off): the model next to a CPU-rendered desktop
#   "N N N": numbers of active sessions to measure at, ascending (default "0 1 5 10"); 0 = the model alone. Sessions are added one at a time up to each N and kept.
#   "C C C": concurrent model requests at each N (default "1 2 4").
#   PROFILE: mixed (default) | heavy | video, as in ramp-desktops.sh (work.sh inside each session). DESKTOP: waydesk-desktop (default) | sway-minimal (sway only: Hyprland on NVIDIA is untested).
# At each N > 0: a WINDOW-second window with the sessions only ("sessions" row), then one WINDOW-second window per C with the model loaded by the same sessions ("combined" rows).
# At N = 0: one window per C with the model alone ("model" rows). Windows are sampled by ai-sample.py on the server while llm-bench.py runs there.
# Criteria, stated before the run and written in the header of the output (this is a comparison, not a ramp to a limit):
#   validity  the client machine busier than STOP_CLIENT_CPU (60) = the row does not count and the run stops
#   validity  the model not wholly in video memory (Ollama's size_vram under its size) = flagged in the note, the numbers are of a model that spilled to the CPU
#   validity  available memory under STOP_AVAIL_MB (1500) or an admission refusal = the run stops there
#   Degradation is read afterwards (ai-summary.py): the sessions' fps and bitrate with the model against the same N without it, and the model's tokens/s with sessions against alone.
# Output: out-LABEL.tsv, raw-LABEL/*.json (every window's sampler and bench output), raw-LABEL/system.txt. Run from this folder; needs SERVER and CLIENT (README.md) and the
# model already pulled and pinned (ai-setup-server.sh). MODEL (default llama3.1:8b-instruct-q4_K_M), WINDOW (90).
LABEL=${1:?label}; GPUMODE=${2:?igpu|rtx|balance|software}; NLIST=${3:-"5 10"}; CLIST=${4:-"1 2"}; AIU=${5:-3}; PROFILE=${6:-mixed}; DESKTOP=${7:-waydesk-desktop}
MODEL=${MODEL:-llama3.1:8b-instruct-q4_K_M}; WINDOW=${WINDOW:-90}; SETTLE=${SETTLE:-60}
STOP_AVAIL_MB=${STOP_AVAIL_MB:-1500}; STOP_CLIENT_CPU=${STOP_CLIENT_CPU:-60}
SERVER=${SERVER:?set SERVER to the machine under test}; CLIENT=${CLIENT:?set CLIENT to the machine that runs the RDP clients}
K=${SSH_KEY:-$HOME/.ssh/id_ed25519}; KH=${KNOWN_HOSTS:-$HOME/.ssh/known_hosts}; SKH=${SERVER_KNOWN_HOSTS:-$KH}
T() { ssh -i $K -o IdentitiesOnly=yes -o UserKnownHostsFile=$SKH -o ServerAliveInterval=15 root@$SERVER "$@"; }
C() { ssh -i $K -o IdentitiesOnly=yes -o UserKnownHostsFile=$KH -o ServerAliveInterval=15 root@$CLIENT "$@"; }
case $DESKTOP in
  sway-minimal|waydesk-desktop) COMP=sway;;
  *) echo "DESKTOP must be waydesk-desktop or sway-minimal" >&2; exit 2;;
esac
OUT=out-$LABEL.tsv; RAW=raw-$LABEL; mkdir -p $RAW

# The cards, as the server sees them: the render node of the first Intel and the first NVIDIA card.
CARDS=$(T "waydesk-admin status --json" | python3 -c '
import json,sys
g=json.load(sys.stdin).get("gpus",[])
n=lambda v: next((x["node"].rsplit("/",1)[-1] for x in g if x["vendor"]==v),"")
print(n("intel"), n("nvidia"))')
INTEL_NODE=${CARDS% *}; NV_NODE=${CARDS#* }
case $GPUMODE in
  igpu)     [ -n "$INTEL_NODE" ] || { echo "no Intel card on the server" >&2; exit 2; }; GPUARG=$INTEL_NODE;;
  rtx)      [ -n "$NV_NODE" ] || { echo "no NVIDIA card on the server" >&2; exit 2; }; GPUARG=$NV_NODE;;
  balance)  GPUARG=balance;;
  software) GPUARG=off;;
  *) echo "unknown GPUMODE $GPUMODE" >&2; exit 2;;
esac

T 'uname -a; echo; head -3 /etc/os-release; echo; lscpu | grep -E "Model name|^CPU\(s\)"; free -m; echo; waydesk-admin status 2>&1 | head -30; echo; cat /etc/default/waydesk 2>/dev/null; echo; nvidia-smi 2>&1 | head -20; echo; ollama --version 2>&1; ollama ps 2>&1; systemctl cat ollama 2>&1 | grep -E "Environment|ExecStart"' > $RAW/system.txt 2>&1
{ printf '# ai-load-insession.sh label=%s gpumode=%s (sessions on %s) model=%s profile=%s desktop=%s window=%ss ai_users=%s (conc is per account) date=%s\n' "$LABEL" "$GPUMODE" "$GPUARG" "$MODEL" "$PROFILE" "$DESKTOP" "$WINDOW" "$AIU" "$(date -u +%FT%TZ)"
  printf '# criteria: STOP_CLIENT_CPU=%s STOP_AVAIL_MB=%s; model must be wholly in video memory; degradation is read afterwards with ai-summary.py\n' "$STOP_CLIENT_CPU" "$STOP_AVAIL_MB"
  printf 'phase\tn\tconc\trequests\terrors\ttps_med\ttps_min\ttps_total\tttft_med_s\tttft_p95_s\tsessions\tfps_med\tfps_p10\tkbps_total\tcpu_busy_pct\tigpu_busy\tigpu_enc\tnv_busy\tnv_enc\tnv_mem_used_MB\tmodel_vram_MB\tmodel_total_MB\tavail_MB\tclient_cpu_pct\tnote\n'; } > $OUT

HAVE=0
add_user() {  # one more active session, pinned as GPUMODE says; leaves HAVE one higher, or returns 1 when it could not start
  local i=$((HAVE + 1)) name=lt$((HAVE + 1)) prof=worker snd='' gimp=0 hv=0
  [ "$PROFILE" = heavy ] && { hv=1; [ $((i % 4)) = 2 ] && gimp=1; }
  if [ "$PROFILE" = video ]; then prof=video; else
    [ $((i % 4)) = 0 ] && prof=video; case $i in 3|7|11) prof=videoaudio; snd=/sound;; esac
  fi
  local J pw uid
  J=$(T "waydesk-admin users add $name --json") || return 1
  pw=$(printf '%s' "$J" | python3 -c 'import json,sys;print(json.load(sys.stdin)["password"])')
  uid=$(printf '%s' "$J" | python3 -c 'import json,sys;print(json.load(sys.stdin)["uid"])')
  T "waydesk-admin users set $name --desktop sway --gpu $GPUARG" >/dev/null || return 1
  [ "$DESKTOP" = sway-minimal ] && T "install -d -o $name -g $name /home/$name/.config/waydesk && install -o $name -g $name /usr/share/waydesk/sway-minimal.conf /home/$name/.config/waydesk/config"
  C "SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy setsid nohup sdl-freerdp3 /v:$SERVER:3389 /u:$name '/p:$pw' /cert:ignore /size:1680x1050 /gfx:AVC420 $snd > /root/client-$LABEL-$i.log 2>&1 < /dev/null &"
  local ok=0 t
  for t in $(seq 1 40); do
    T "waydesk-admin sessions list 2>/dev/null" | awk '{print $1}' | grep -qx "$name" && { ok=1; break; }
    sleep 3
  done
  [ $ok = 1 ] || return 1
  sleep 25
  T "wl=\$(ls /run/user/$uid | grep -E '^wayland-[0-9]+\$' | head -1); sock=\$(ls /run/user/$uid/sway-ipc.*.sock 2>/dev/null | head -1); setsid nohup runuser -u $name -- env XDG_RUNTIME_DIR=/run/user/$uid WAYLAND_DISPLAY=\${wl:-wayland-1} DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$uid/bus SWAYSOCK=\$sock COMPOSITOR=$COMP PATH=/usr/lib/waydesk/bin:/usr/bin:/bin HOME=/home/$name USER=$name HEAVY=$hv GIMP=$gimp /srv/load/work.sh $prof > /root/load/work-$LABEL-$i.log 2>&1 < /dev/null &"
  HAVE=$i
}

window() {  # window PHASE N CONC : sample the server for WINDOW s (and run the model load when CONC > 0), the client machine's CPU at the same time; append a row
  local phase=$1 n=$2 conc=$3 tag=$1-n$2-c$3
  ( C "python3 -c \"
import time
def r():
    f=list(map(int,open('/proc/stat').readline().split()[1:])); return sum(f),f[3]+f[4]
a=r(); time.sleep($WINDOW); b=r(); print(round(100*(1-(b[1]-a[1])/(b[0]-a[0])),1))\"" > .clientcpu.$$ ) &
  if [ "$conc" -gt 0 ]; then
    # the first AIU sessions each call the model as their own account; a status snapshot in the middle of the window shows the AI column
    T "rm -f /tmp/ins-lt*.json /tmp/mid.json; for k in \$(seq 1 $AIU); do (runuser -u lt\$k -- env OLLAMA_URL=http://127.0.0.1:11434 python3 /srv/load/llm-bench.py $conc $WINDOW $MODEL > /tmp/ins-lt\$k.json 2>/dev/null &) ; done; (sleep $((WINDOW / 2)); waydesk-admin status --json > /tmp/mid.json 2>/dev/null) & python3 /srv/load/ai-sample.py $WINDOW > /tmp/sample.json; wait; for k in \$(seq 1 $AIU); do for t in \$(seq 1 90); do [ -s /tmp/ins-lt\$k.json ] && break; sleep 1; done; done; cat /tmp/ins-lt*.json" > $RAW/$tag.bench.json
    T "cat /tmp/mid.json" > $RAW/$tag.mid.json
  else
    T "python3 /srv/load/ai-sample.py $WINDOW" > $RAW/$tag.sample.json
  fi
  wait
  local ccpu; ccpu=$(cat .clientcpu.$$); rm -f .clientcpu.$$
  [ "$conc" -gt 0 ] && T "cat /tmp/sample.json" > $RAW/$tag.sample.json
  local avail; avail=$(T "free -m | awk '/^Mem:/ {print \$7}'")
  T "waydesk-admin status --json" > $RAW/$tag.status.json 2>/dev/null
  python3 - "$phase" "$n" "$conc" "$avail" "$ccpu" "$RAW/$tag" >> $OUT <<'PY'
import sys, json
phase, n, conc, avail, ccpu, base = sys.argv[1:]
def load(p):
    try: return json.load(open(p))
    except Exception: return {}
def load_bench(p):
    import statistics
    rows = []
    try:
        for l in open(p):
            if l.strip(): rows.append(json.loads(l))
    except Exception:
        pass
    ok = [r for r in rows if r.get('tps_med') is not None]
    if not rows: return {}
    if not ok: return {'error': str(rows[0].get('error', 'no result'))}
    return {'requests': sum(r['requests'] for r in rows), 'errors': sum(r['errors'] for r in rows), 'tps_med': round(statistics.median(r['tps_med'] for r in ok), 1),
            'tps_min': min(r['tps_min'] for r in ok), 'tps_total': round(sum(r['tps_total'] for r in ok), 1), 'ttft_med': round(statistics.median(r['ttft_med'] for r in ok), 2),
            'ttft_p95': max(r['ttft_p95'] for r in ok), 'accounts': len(ok)}
b = load_bench(base + '.bench.json'); s = load(base + '.sample.json')
mid = load(base + '.mid.json')
ai_col = ' '.join('%s=%s' % (u['name'], u['ai_connections']) for u in mid.get('users', []) if u.get('ai_connections'))
if ai_col: note_ai = 'AI column: ' + ai_col
else: note_ai = ''
if b.get('accounts'): note_ai = ('%s accounts; ' % b['accounts']) + note_ai
gpus = s.get('gpus', {})
card = lambda vendor: next((g for g in gpus.values() if g.get('vendor') == vendor), {})
ig, nv = card('intel'), card('nvidia')
note = []
if b.get('error'): note.append(b['error'])
if s.get('warnings'): note.append('session in software: ' + '; '.join(w.split(' renders')[0] for w in s['warnings']))
mv, mt = s.get('model_vram_mb'), s.get('model_total_mb')
if conc != '0' and mv is not None and mt and mv < 0.98 * mt: note.append('MODEL PARTLY ON CPU (%s of %s MB in video memory)' % (mv, mt))
if conc != '0' and mv is None: note.append('model not seen by Ollama')
g = lambda d, k: '-' if d.get(k) is None else d[k]
if note_ai: note.append(note_ai)
print('\t'.join(map(str, [phase, n, conc, g(b, 'requests'), g(b, 'errors'), g(b, 'tps_med'), g(b, 'tps_min'), g(b, 'tps_total'), g(b, 'ttft_med'), g(b, 'ttft_p95'),
    g(s, 'sessions'), g(s, 'fps_med'), g(s, 'fps_p10'), g(s, 'kbps_total'), g(s, 'cpu_busy_pct'), g(ig, 'busy'), g(ig, 'enc'), g(nv, 'busy'), g(nv, 'enc'), g(nv, 'mem_used_mb'),
    g(s, 'model_vram_mb'), g(s, 'model_total_mb'), avail, ccpu, '; '.join(note) or 'ok'])))
PY
  tail -1 $OUT
  # validity: the criteria stated in the header
  python3 -c "import sys; sys.exit(0 if float('${ccpu:-0}')<$STOP_CLIENT_CPU else 1)" || { echo "stop: client saturated (${ccpu} % at $tag): this row and later ones do not count" | tee -a $OUT; return 1; }
  [ "${avail:-99999}" -lt "$STOP_AVAIL_MB" ] && { echo "stop: memory (available ${avail} MB at $tag)" | tee -a $OUT; return 1; }
  return 0
}

for n in $NLIST; do
  while [ $HAVE -lt "$n" ]; do
    add_user || { echo "stop: could not start session $((HAVE + 1)) (refused or failed; client log on the client machine: client-$LABEL-$((HAVE + 1)).log)" | tee -a $OUT; echo "done $(date +%T)" >> $OUT; exit 1; }
  done
  [ "$n" -ge "$AIU" ] || { echo "N=$n is less than AIU=$AIU: skipped"; continue; }
  sleep $SETTLE
  window sessions $n 0 || { echo "done $(date +%T)" >> $OUT; exit 1; }
  for c in $CLIST; do
    window combined $n $c || { echo "done $(date +%T)" >> $OUT; exit 1; }
  done
done
echo "done $(date +%T)" >> $OUT
