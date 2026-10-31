#!/usr/bin/env python3
# ai-summary.py out-LABEL.tsv [out-OTHER.tsv ...] : what the model did to the sessions, and the sessions to the model, from the rows of ai-load.sh.
#   sessions: fps and bitrate with the model running (a "combined" row) against the same N without it (the "sessions" row), as a percentage of the latter
#   model:    tokens/s of one request and in all with N sessions against the model alone (N = 0) at the same concurrency
#   VRAM:     what each session takes on the NVIDIA card = (card memory used at N, model loaded, no requests) minus (card memory used with the model alone) over N.
#             Only meaningful when the sessions are on that card (GPUMODE rtx) and the model stays loaded (it does: OLLAMA_KEEP_ALIVE=-1).
# Rows after a "stop:" line (a failed validity criterion) and rows flagged MODEL PARTLY ON CPU are listed but left out of the comparisons.
import sys


def num(x):
    try:
        return float(x)
    except ValueError:
        return None


def rows_of(path):
    head, rows, stopped = None, [], False
    for line in open(path):
        line = line.rstrip('\n')
        if line.startswith('#') or not line.strip():
            continue
        if line.startswith('stop:'):
            stopped = True
            print('  ' + line)
            continue
        if line.startswith('done'):
            continue
        f = line.split('\t')
        if f[0] == 'phase':
            head = f
            continue
        if head and len(f) == len(head):
            r = dict(zip(head, f))
            r['_valid'] = not stopped and 'PARTLY ON CPU' not in r['note']
            rows.append(r)
    return rows


def pct_change(new, old):
    return None if new is None or not old else 100.0 * (new - old) / old


def show(v, suffix='%'):
    return '-' if v is None else f'{v:+.0f}{suffix}'


def summarise(path):
    print(f'\n== {path}')
    rows = rows_of(path)
    base = {int(r['n']): r for r in rows if r['phase'] == 'sessions' and r['_valid']}
    alone = {int(r['conc']): r for r in rows if r['phase'] == 'model' and r['_valid']}
    print('\nSessions with the model against the same sessions without it')
    print(f"{'N':>3} {'conc':>4} | {'fps med':>8} {'(alone)':>8} {'change':>7} | {'fps p10':>8} {'change':>7} | {'kbps':>7} {'change':>7} | {'cpu %':>6} {'(alone)':>7}")
    for r in rows:
        if r['phase'] != 'combined':
            continue
        b = base.get(int(r['n']))
        flag = '' if r['_valid'] else '  (not counted: ' + r['note'] + ')'
        if not b:
            print(f"{r['n']:>3} {r['conc']:>4} | no baseline window{flag}")
            continue
        print(f"{r['n']:>3} {r['conc']:>4} | {r['fps_med']:>8} {b['fps_med']:>8} {show(pct_change(num(r['fps_med']), num(b['fps_med']))):>7} | {r['fps_p10']:>8} {show(pct_change(num(r['fps_p10']), num(b['fps_p10']))):>7} | "
              f"{r['kbps_total']:>7} {show(pct_change(num(r['kbps_total']), num(b['kbps_total']))):>7} | {r['cpu_busy_pct']:>6} {b['cpu_busy_pct']:>7}{flag}")
    print('\nThe model with sessions against the model alone')
    print(f"{'N':>3} {'conc':>4} | {'tok/s one':>9} {'(alone)':>8} {'change':>7} | {'tok/s all':>9} {'change':>7} | {'ttft med':>8} {'(alone)':>8} | {'ttft p95':>8} | {'nv busy':>7} {'nv enc':>6}")
    for r in rows:
        if r['phase'] != 'combined':
            continue
        a = alone.get(int(r['conc']))
        flag = '' if r['_valid'] else '  (not counted: ' + r['note'] + ')'
        if not a:
            print(f"{r['n']:>3} {r['conc']:>4} | no model-alone window{flag}")
            continue
        print(f"{r['n']:>3} {r['conc']:>4} | {r['tps_med']:>9} {a['tps_med']:>8} {show(pct_change(num(r['tps_med']), num(a['tps_med']))):>7} | {r['tps_total']:>9} {show(pct_change(num(r['tps_total']), num(a['tps_total']))):>7} | "
              f"{r['ttft_med_s']:>8} {a['ttft_med_s']:>8} | {r['ttft_p95_s']:>8} | {r['nv_busy']:>7} {r['nv_enc']:>6}{flag}")
    ref = next(iter(alone.values()), None)
    mem0 = num(ref['nv_mem_used_MB']) if ref else None
    print('\nVideo memory of a session on the NVIDIA card (model loaded, no requests)')
    if mem0 is None:
        print('  no model-alone row with a memory reading: not computed')
    else:
        for n, b in sorted(base.items()):
            m = num(b['nv_mem_used_MB'])
            print(f"  N={n:>2}: card {b['nv_mem_used_MB']} MB, model alone {mem0:.0f} MB -> " + ('-' if m is None or n == 0 else f'{(m - mem0) / n:.0f} MB a session'))
    warn = [r for r in rows if 'session in software' in r['note']]
    if warn:
        print('\nRows where a session fell back to software:', ', '.join(f"{r['phase']} N={r['n']} conc={r['conc']}" for r in warn))


if __name__ == '__main__':
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    for p in sys.argv[1:]:
        summarise(p)
