# AI-load test: desktops and a local model on one server (plan, 2026-10-31)

Question: with a model served from the server's GPU, what does it do to the desktops, and what do the desktops do to the model? Driver: `ai-load.sh`; read the results with `ai-summary.py`.
Companion design: `docs/GPU-PLACEMENT-DESIGN.md` (which GPU a session gets, and a GPU kept for the model). Not run yet: written while waiting for the RTX server.

## Machine
Hetzner auction i5-13500 with Intel UHD 770 and RTX 4000 SFF Ada (20 GB), Debian 13, Waydesk 0.4.0 plus the `enterprise` branch (`users set --gpu`, `gpu place`, `status` per card). NVIDIA driver 615 or newer from NVIDIA's `debian13` repository
(`nvidia-open`), the udev rule for the render nodes, and the GPU-aware `/etc/chromium.d/waydesk` (see `docs/GPU-PLAN.md`). Sway desktops only (Hyprland on NVIDIA is untested). Client machine and driver as in `README.md`
(client on a separate cloud server, 8 dedicated vCPU; the Mac plugged in with `caffeinate -dis`).

Setup, once: the desktop side of `README.md`, then `ai-setup-server.sh` (Ollama pinned to the RTX by UUID, localhost only, models kept loaded, 4 parallel requests; pulls an 8B and a 14B at Q4).
Before any run: `python3 /srv/load/llm-bench.py 1 20 MODEL` by hand (tokens/s sane, `ollama ps` says 100 % GPU, `nvidia-smi --query-compute-apps=pid,name,used_memory --format=csv` shows Ollama on the RTX only).

## What is varied
| Variable | Values |
|---|---|
| Where the sessions render (`GPUMODE`) | `igpu` (UHD 770), `rtx` (same card as the model), `balance` (the placement; reserve the RTX with `WAYDESK_GPU_RESERVE` to repeat `igpu` through the placement itself), `software` |
| Active sessions (N) | 0 (model alone), 1, 5, 10; more if the first run shows room |
| Concurrent model requests (C) | 1, 2, 4 |
| Model | 8B Q4 first; 14B Q4 as the second pass (`MODEL=...`) |
| Session load (`PROFILE`) | `mixed`; `video` as the worst case for the encoder |

Order: 8B with `igpu`, `rtx`, `software`; then `balance`; then the 14B with the first two. Between runs `./cleanup-remote.sh`, `loginctl` without sessions, `free -m` back to the baseline, and `ollama ps` showing the same model loaded.

## Criteria, stated before the run (also in each `out-*.tsv`)
This is a comparison at fixed points, not a ramp to a limit.
- **Validity:** client machine under `STOP_CLIENT_CPU=60` %, available memory over `STOP_AVAIL_MB=1500`, no admission refusal; the model wholly in video memory (a row where Ollama's `size_vram` is under its `size` is flagged and left out of the comparisons).
- **Degradation, read afterwards:** the sessions' fps (median and 10th percentile) and bitrate with the model against the same N without it; the model's tokens/s (one request and in all) and time to first token with sessions against alone.
  No threshold is fixed in advance for "too much"; the table is the result. Anything under about 15 fps median for a session is visibly poor in the earlier video runs, and a time to first token over 2 s feels slow: use those as reading aids, not as pass marks.
- **Memory:** video memory a session takes on the RTX = (card memory with N sessions and the model loaded) minus (card memory with the model alone), over N. Meaningful for `rtx` only.

## What would settle the open points of the design
1. VRAM per session on NVIDIA (above) decides `WAYDESK_GPU_MIN_FREE_VRAM_MB` and whether the RTX can host sessions and a 14B at once.
2. `igpu` against `rtx` rows at the same N and C show whether pinning matters, and so whether the reserve is worth its complexity.
3. `balance` rows show whether the 4:1 weight puts the sessions where they do least harm. Check `waydesk-admin status` between steps: sessions per card, and no `Warning` lines (a session that fell back to software shows in the row's note).
4. NVENC session cap: with `rtx` and `video`, the first N at which a session reports `openh264` instead of `nvenc` in `status --json` (the encoder column) is the cap on this card.

## Raw data
`out-LABEL.tsv` (row per window, criteria in the header), `raw-LABEL/*.json` (sampler, bench and `status --json` per window), `raw-LABEL/system.txt` (versions, driver, Ollama's environment). Commit the tsv and json with the scripts, next to the article.

## Not in this test
Per-user limits on the model; two models at once; vLLM (Ollama batches by `OLLAMA_NUM_PARALLEL`, which is enough to see the contention; a vLLM run would show higher total throughput at the same contention); long prompts (the prompt is the same short one each time, 256 tokens out).
