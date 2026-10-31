#!/usr/bin/env python3
# sample.py SECONDS : CPU split by group, GPU engine busy (intel_gpu_top), memory, over SECONDS; prints one TSV row of values
import os, sys, time, json, subprocess, collections
T = int(sys.argv[1])
def cpu_total():
    f = open('/proc/stat').readline().split()[1:]; v = list(map(int, f)); return sum(v), v[3] + v[4]
def procs():
    d = {}
    for p in os.listdir('/proc'):
        if not p.isdigit(): continue
        try:
            s = open(f'/proc/{p}/stat').read(); c = s[s.rindex(')') + 2:].split(); n = s[s.index('(') + 1:s.rindex(')')]
            d[p] = (n, int(c[11]) + int(c[12]))
        except Exception: pass
    return d
def group(n):
    if n.startswith('waydesk') or n == 'waydesk-server': return 'waydesk'
    if n.startswith('sway') or n.lower().startswith(('hyprland', 'start-hyprland', '.hyprland', 'cosmic-comp')): return 'sway'  # the compositor, whichever it is
    if 'chrom' in n or n in ('chromium',): return 'chromium'
    if n in ('dms', 'qs', 'quickshell') or 'dms' in n or n.startswith('cosmic-'): return 'shell'
    if n.startswith('sdl-freerdp') or n.startswith('xfreerdp'): return 'CLIENTS'
    if n == 'foot': return 'foot'
    if n.startswith('sshd'): return 'sshd'
    if n in ('python3', 'intel_gpu_top', 'timeout', 'journalctl', 'bash', 'sh'): return 'harness'
    return 'other'
gpuf = open('/tmp/igt.%d.json' % os.getpid(), 'w')  # a file, not a pipe: with many clients a sample is large and a full pipe blocks the tool
gpu = subprocess.Popen(['timeout', '-s', 'INT', str(T), 'intel_gpu_top', '-J', '-s', '1000'], stdout=gpuf, stderr=subprocess.DEVNULL, text=True)
t0, i0 = cpu_total(); p0 = procs(); time.sleep(T)
t1, i1 = cpu_total(); p1 = procs()
tick = os.sysconf('SC_CLK_TCK'); ncpu = os.cpu_count()
g = collections.Counter()
for p, (n, t) in p1.items():
    if p in p0: g[group(n)] += (t - p0[p][1])
out = {'cpu_busy_pct': round(100 * (1 - (i1 - i0) / (t1 - t0)), 1)}
for k, v in sorted(g.items()): out['cpu_' + k] = round(100 * v / tick / T / ncpu * 1, 1)  # percent of the whole machine
import re
try:
    gpu.wait(timeout=15); gpuf.close(); txt = open(gpuf.name).read(); os.unlink(gpuf.name)
    for eng in ('Render/3D', 'Video', 'VideoEnhance', 'Blitter'):
        v = [float(x) for x in re.findall(r'"%s": \{\s*"busy": ([0-9.]+)' % re.escape(eng), txt)][1:]
        if v: out['gpu_' + eng.split('/')[0]] = round(sum(v) / len(v), 1)
    f = [float(x) for x in re.findall(r'"actual": ([0-9.]+)', txt)][1:]
    if f: out['gpu_MHz'] = round(sum(f) / len(f))
    # who used the render engine: the `clients` of each sample (intel_gpu_top -J), summed by program name, averaged over the samples
    import json as _json
    arr = _json.loads(re.sub(r"\}\s*\n\s*\{", "},{", ("[" + txt.strip().lstrip("[")).rstrip().rstrip(",").rstrip("]") + "]"))[1:]
    agg = collections.Counter()
    for d in arr:
        for c in (d.get("clients") or {}).values():
            agg[c.get("name", "?")] += float(((c.get("engine-classes") or {}).get("Render/3D") or {}).get("busy", 0))
    out['gpu_top'] = ",".join("%s:%.1f" % (k, v / max(len(arr), 1)) for k, v in agg.most_common(4) if v > 0)
    w = [float(x) for x in re.findall(r'"Package": ([0-9.]+)', txt)][1:]
    if w: out['pkg_W'] = round(sum(w) / len(w), 1)
except Exception as ex: out['gpu_err'] = str(ex)[:40]
print(json.dumps(out))
