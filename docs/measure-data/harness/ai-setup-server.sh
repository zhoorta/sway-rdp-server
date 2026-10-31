#!/bin/bash
# ai-setup-server.sh : the model side of the AI-load test, on the server, as root, once (after the desktop side of README.md is in place).
#   MODELS  space-separated Ollama tags to pull (default: llama3.1:8b-instruct-q4_K_M qwen2.5:14b-instruct-q4_K_M; check the tags exist: `ollama pull` fails on a wrong one)
#   MODEL_GPU  the NVIDIA card to keep the model on: its UUID (default: the first NVIDIA card in `waydesk-admin status --json`)
#   PARALLEL   requests Ollama serves at once (default 4: the largest concurrency of the test)
# What it does: installs Ollama (the vendor's installer, which you can read first: https://ollama.com/install.sh), pins the service to one card with CUDA_VISIBLE_DEVICES=<UUID>
# (the UUID does not depend on how CUDA counts the devices), keeps it on localhost, keeps models loaded, pulls the models, and copies the load scripts to /srv/load.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
MODELS=${MODELS:-"llama3.1:8b-instruct-q4_K_M qwen2.5:14b-instruct-q4_K_M"}
PARALLEL=${PARALLEL:-4}
UUID=${MODEL_GPU:-$(waydesk-admin status --json | python3 -c 'import json,sys; print(next((g.get("uuid") or "" for g in json.load(sys.stdin).get("gpus",[]) if g["vendor"]=="nvidia"), ""))')}
[ -n "$UUID" ] || { echo "no NVIDIA card with a UUID in waydesk-admin status --json (is the driver, 615 or newer, installed? nvidia-smi -L)" >&2; exit 1; }
command -v ollama >/dev/null || curl -fsSL https://ollama.com/install.sh | sh
mkdir -p /etc/systemd/system/ollama.service.d
cat > /etc/systemd/system/ollama.service.d/waydesk-test.conf <<EOF
[Service]
Environment="CUDA_VISIBLE_DEVICES=$UUID"
Environment="OLLAMA_HOST=127.0.0.1:11434"
Environment="OLLAMA_NUM_PARALLEL=$PARALLEL"
Environment="OLLAMA_MAX_LOADED_MODELS=1"
Environment="OLLAMA_KEEP_ALIVE=-1"
EOF
systemctl daemon-reload && systemctl enable --now ollama && systemctl restart ollama
for i in $(seq 1 30); do curl -fs http://127.0.0.1:11434/api/version >/dev/null && break; sleep 1; done
for m in $MODELS; do ollama pull "$m"; done
mkdir -p /srv/load && cp "$HERE"/llm-bench.py "$HERE"/ai-sample.py /srv/load/ && chmod 755 /srv/load/llm-bench.py /srv/load/ai-sample.py
echo
echo "Ollama pinned to $UUID, models: $MODELS"
echo "Check, before any run:  OLLAMA_URL=http://127.0.0.1:11434 python3 /srv/load/llm-bench.py 1 20 ${MODELS%% *}   (tokens/s and whether 'ollama ps' says 100% GPU)"
echo "and that the card is the right one:  nvidia-smi --query-compute-apps=pid,name,used_memory --format=csv"
