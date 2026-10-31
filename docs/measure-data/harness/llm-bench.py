#!/usr/bin/env python3
# llm-bench.py CONCURRENCY SECONDS MODEL : load on a local Ollama server, one line of JSON out.
# CONCURRENCY workers each send the same prompt over and over (streaming, 256 tokens at most, temperature 0) for SECONDS; a request that started before the end finishes.
# One warm-up request first (not counted; its load time is reported), so the model is resident and the numbers are not the first load.
#   tps_med / tps_min   tokens a second of one request (what one user sees): the model's own count over its own generation time
#   tps_total           all tokens of the window over the window's length (what the card delivers to everybody)
#   ttft_med / ttft_p95 seconds from sending the request to the first token
# OLLAMA_URL (default http://127.0.0.1:11434). Standard library only.
import json, os, statistics, sys, threading, time, urllib.request

URL = os.environ.get('OLLAMA_URL', 'http://127.0.0.1:11434')
PROMPT = 'Explain, in about 200 words, how a hash table handles collisions, then give a short Python example.'
NUM_PREDICT = int(os.environ.get('NUM_PREDICT', '256'))


def request(model, num_predict):
    """One streamed generation: (ttft seconds, tokens, generation seconds, load seconds, error)."""
    body = json.dumps({'model': model, 'prompt': PROMPT, 'stream': True, 'keep_alive': -1,
                       'options': {'num_predict': num_predict, 'temperature': 0}}).encode()
    req = urllib.request.Request(URL + '/api/generate', data=body, headers={'Content-Type': 'application/json'})
    t0 = time.time()
    ttft, final = None, None
    try:
        with urllib.request.urlopen(req, timeout=600) as r:
            for line in r:
                if not line.strip():
                    continue
                d = json.loads(line)
                if ttft is None and d.get('response'):
                    ttft = time.time() - t0
                if d.get('done'):
                    final = d
    except Exception as e:  # a refused or dropped connection is a result, not a crash
        return None, 0, 0.0, 0.0, str(e)[:80]
    if not final or ttft is None:
        return None, 0, 0.0, 0.0, 'no answer'
    return ttft, final.get('eval_count', 0), final.get('eval_duration', 0) / 1e9, final.get('load_duration', 0) / 1e9, None


def pct(values, p):
    if not values:
        return None
    s = sorted(values)
    return s[min(len(s) - 1, int(round(p / 100 * (len(s) - 1))))]


def main():
    conc, seconds, model = int(sys.argv[1]), float(sys.argv[2]), sys.argv[3]
    _, _, _, load_s, err = request(model, 8)
    if err:
        print(json.dumps({'conc': conc, 'error': 'warm-up failed: ' + err}))
        return 1
    results, lock = [], threading.Lock()
    start = time.time()
    stop_at = start + seconds

    def worker():
        while time.time() < stop_at:
            r = request(model, NUM_PREDICT)
            with lock:
                results.append((r, time.time()))

    threads = [threading.Thread(target=worker) for _ in range(conc)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    wall = max(r[1] for r in results) - start if results else seconds
    ok = [r[0] for r in results if r[0][4] is None and r[0][2] > 0]
    tps = [r[1] / r[2] for r in ok]
    ttft = [r[0] for r in ok]
    out = {
        'conc': conc, 'seconds': seconds, 'model': model, 'load_s': round(load_s, 2),
        'requests': len(ok), 'errors': len(results) - len(ok),
        'tps_med': round(statistics.median(tps), 1) if tps else None,
        'tps_min': round(min(tps), 1) if tps else None,
        'tps_total': round(sum(r[1] for r in ok) / wall, 1) if ok else None,
        'ttft_med': round(statistics.median(ttft), 2) if ttft else None,
        'ttft_p95': round(pct(ttft, 95), 2) if ttft else None,
    }
    print(json.dumps(out))
    return 0


if __name__ == '__main__':
    sys.exit(main())
