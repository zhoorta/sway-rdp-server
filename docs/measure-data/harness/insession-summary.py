#!/usr/bin/env python3
# insession-summary.py out-LABEL.tsv : the rows of ai-load-insession.sh as a table. conc is requests per account; three accounts call the model from their own sessions.
import sys
rows = [l.rstrip('\n').split('\t') for l in open(sys.argv[1]) if l.strip() and not l.startswith(('#', 'stop:', 'done'))]
head, rows = rows[0], [dict(zip(rows[0], r)) for r in rows[1:] if len(r) == len(rows[0])]
base = {r['n']: r for r in rows if r['phase'] == 'sessions'}
print(f"{'N':>3} {'per acct':>8} {'in flight':>9} | {'tok/s each':>10} {'slowest':>8} {'tok/s all':>9} | {'ttft med':>8} {'ttft p95':>8} | {'fps med':>7} {'(no AI)':>7} {'p10':>5} | {'cpu %':>6} | AI column")
for r in rows:
    if r['phase'] != 'combined': continue
    b = base.get(r['n'], {})
    accts = int(r['note'].split(' accounts')[0]) if ' accounts' in r['note'] else 0
    print(f"{r['n']:>3} {r['conc']:>8} {accts * int(r['conc']):>9} | {r['tps_med']:>10} {r['tps_min']:>8} {r['tps_total']:>9} | {r['ttft_med_s']:>8} {r['ttft_p95_s']:>8} | {r['fps_med']:>7} {b.get('fps_med', '-'):>7} {r['fps_p10']:>5} | {r['cpu_busy_pct']:>6} | {r['note'].split('AI column: ')[-1]}")
