#!/usr/bin/env python3
# ai-sample.py SECONDS [EVERY] : what the sessions and the cards do over SECONDS, one line of JSON. Run on the server, next to llm-bench.py.
# Every EVERY seconds (default 5) it reads `waydesk-admin status --json` (which itself samples for a second) and Ollama's /api/ps, and keeps:
#   sessions        median number of sessions with fresh statistics
#   fps_med, fps_p10  frames a second of the sessions: the median of all readings, and the 10th percentile (the worst-served tenth)
#   kbps_total      median of the sum of the sessions' bitrates
#   cpu_busy_pct    busy share of all threads over the whole window (/proc/stat)
#   gpus            per card (keyed by node): vendor, model, busy, encoder %, memory used (median of the samples)
#   model_vram_mb   what Ollama says the model holds in video memory (median); model_total_mb its whole size (a model that is partly on the CPU is not a result)
#   warnings        the `Warning` lines of status (a session in software although a GPU was asked for), once each
# Memory used by the sessions on a card = the card's used memory minus model_vram_mb (when the model is on that card).
import json, os, statistics, subprocess, sys, time, urllib.request

T = float(sys.argv[1])
EVERY = float(sys.argv[2]) if len(sys.argv) > 2 else 5.0
URL = os.environ.get('OLLAMA_URL', 'http://127.0.0.1:11434')


def cpu():
    v = list(map(int, open('/proc/stat').readline().split()[1:]))
    return sum(v), v[3] + v[4]


def status():
    try:
        return json.loads(subprocess.run(['waydesk-admin', 'status', '--json'], capture_output=True, text=True, timeout=30).stdout)
    except Exception:
        return None


def ollama_ps():
    try:
        with urllib.request.urlopen(URL + '/api/ps', timeout=5) as r:
            models = json.load(r).get('models', [])
        return sum(m.get('size_vram', 0) for m in models) // 1048576, sum(m.get('size', 0) for m in models) // 1048576
    except Exception:
        return None, None


def pct(values, p):
    s = sorted(values)
    return s[min(len(s) - 1, int(round(p / 100 * (len(s) - 1))))] if s else None


def med(values):
    return round(statistics.median(values), 1) if values else None


t0, i0 = cpu()
end = time.time() + T
readings, per_sample_kbps, counts, vram, total = [], [], [], [], []
gpus, warnings = {}, []
samples = 0
while True:
    s = status()
    if s:
        samples += 1
        fresh = [u['video'] for u in s.get('users', []) if u.get('video') and u['video'].get('current') and u['video'].get('fps') is not None]
        readings += [v['fps'] for v in fresh]
        per_sample_kbps.append(sum(v.get('kbps') or 0 for v in fresh))
        counts.append(len(fresh))
        for g in s.get('gpus', []):
            e = gpus.setdefault(g['node'], {'vendor': g.get('vendor'), 'model': g.get('model'), 'busy': [], 'enc': [], 'mem': []})
            for key, field in (('busy', 'busy_percent'), ('enc', 'encoder_percent'), ('mem', 'mem_used_mb')):
                if g.get(field) is not None:
                    e[key].append(g[field])
        for w in s.get('warnings', []):
            if w not in warnings:
                warnings.append(w)
    used, whole = ollama_ps()
    if used is not None:
        vram.append(used)
        total.append(whole)
    left = end - time.time()
    if left <= 0:
        break
    time.sleep(min(EVERY, left))
t1, i1 = cpu()
out = {
    'window_s': T, 'samples': samples,
    'cpu_busy_pct': round(100 * (1 - (i1 - i0) / (t1 - t0)), 1) if t1 > t0 else None,
    'sessions': med(counts), 'fps_med': med(readings), 'fps_p10': round(pct(readings, 10), 1) if readings else None,
    'kbps_total': med(per_sample_kbps),
    'gpus': {n: {'vendor': g['vendor'], 'model': g['model'], 'busy': med(g['busy']), 'enc': med(g['enc']), 'mem_used_mb': med(g['mem'])} for n, g in gpus.items()},
    'model_vram_mb': med(vram), 'model_total_mb': med(total),
    'warnings': warnings,
}
print(json.dumps(out))
