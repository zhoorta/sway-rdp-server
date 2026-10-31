# The harness of the GPU measurements (2026-10-28)

What produced the tables next to this folder (`docs/MEASUREMENTS.md`, "A GPU: heavy active users ..."). It is what was used, cleaned of our server addresses; it needs adapting to your machines.

**Two machines.** The *server* under test (Waydesk installed, root over SSH) and a *client machine* in the same data centre that runs the RDP clients (`sdl-freerdp3`, from the `freerdp3-sdl` package; 8 vCPU was
enough for 24 clients). The drivers run on a third machine (a laptop) and drive both over SSH, so they must keep running: no sleep, and a connection that stays up. The server's RDP port has to be reachable from the
client machine and from nowhere else: `waydesk-admin access custom --bind SERVER-ADDRESS:3389 --allow CLIENT-ADDRESS`, then `waydesk-admin access local` when you are done.

```
export SERVER=203.0.113.10 CLIENT=203.0.113.20     # the machine under test, the machine that runs the clients
export SSH_KEY=~/.ssh/id_ed25519 KNOWN_HOSTS=~/.ssh/known_hosts
```

**On the server**, once (as root):

```
mkdir -p /srv/load /root/load /srv/capacity
cp page.html video.html video-audio.html gl.html term.sh work.sh sample.py /srv/load/ && chmod 755 /srv/load/*.sh /srv/load/sample.py
cp cleanup.sh fps.sh apptest.sh /root/load/ && chmod 755 /root/load/*.sh
apt install intel-gpu-tools wtype libreoffice-calc gimp imagemagick ffmpeg foot chromium   # intel_gpu_top only for Intel; the rest for the workload
flatpak install --system flathub com.visualstudio.code
python3 -c 'import random; random.seed(7); print(",".join("col%d"%i for i in range(10))); [print(",".join(str(round(random.uniform(0,10000),2)) for _ in range(10))) for _ in range(300000)]' > /srv/capacity/data.csv
(cd /srv/capacity && soffice --headless --convert-to ods data.csv)        # Calc opens the .ods without the import dialog
cp -r /usr/lib/python3.13 /srv/capacity/tree                                # a 28 MB source tree for VS Code
convert -size 4000x3000 plasma:fractal -blur 0x1 /srv/load/img.jpg           # for GIMP
ffmpeg -f lavfi -i testsrc2=size=1280x720:rate=30 -t 10 -c:v libx264 -pix_fmt yuv420p /srv/load/clip.mp4
ffmpeg -f lavfi -i testsrc2=size=1280x720:rate=30 -f lavfi -i "aevalsrc=0.2*sin(2*PI*440*t)|0.2*sin(2*PI*554*t):s=48000" -t 10 -c:v libx264 -pix_fmt yuv420p -c:a aac /srv/load/clip-audio.mp4
chmod -R a+rX /srv/load /srv/capacity
```

On the client machine: `apt install freerdp3-sdl`.

**What is what.**

| File | What it does |
|---|---|
| `ramp-remote.sh MAX LABEL` | adds one active heavy user at a time (account, an RDP client on `$CLIENT`, the workload in the session), measures after each, stops at MAX, a refusal, 97 % CPU or low memory; writes `out-LABEL.tsv` |
| `ramp-desktops.sh MAX LABEL DESKTOP [PROFILE]` | the same ramp for comparing desktops (sway-minimal, waydesk-desktop, omarchy, hyprland), with stop criteria stated in the header, raw `status --json` per step, fps/bitrate columns, client-machine CPU; plan: `CAPACITY-PLAN.md` |
| `ramp.sh MAX LABEL` | the same with the clients on the server itself (the first, lighter run; run it on the server) |
| `work.sh PROFILE` | the workload of one user, inside its session: `worker`, `video`, `videoaudio`; `HEAVY=1` adds Calc, VS Code and keystrokes, `GIMP=1` adds GIMP |
| `sample.py SECONDS` | CPU by group of programs, GPU engines (`intel_gpu_top`), clock and power, as one line of JSON |
| `ai-load.sh LABEL GPUMODE ["N.."] ["C.."]` | desktops and a local model together: per number of sessions (N) a window with the sessions only and one per number of concurrent requests (C) with the model on top; tokens/s, time to first token, the sessions' fps and bitrate, both cards, video memory; plan: `AI-LOAD-PLAN.md` |
| `ai-setup-server.sh` | the model side, once: Ollama pinned to the NVIDIA card by UUID, localhost only, models pulled, the load scripts copied to `/srv/load` |
| `llm-bench.py C SECONDS MODEL`, `ai-sample.py SECONDS` | what `ai-load.sh` runs on the server: the model load (tokens/s, time to first token) and the sampler (sessions, cards, video memory, warnings); one line of JSON each |
| `ai-summary.py out-LABEL.tsv` | what the model did to the sessions and the sessions to the model, and the video memory of a session |
| `run-webgl.sh` | one full-screen WebGL page per user, 1 to 12 users, GPU on, then GPU off; reads each page's own frame rate (`fps.sh`) |
| `run-fallback.sh` | one WebGL user with the GPU on, then with VAAPI made unavailable (`LIBVA_DRIVER_NAME`) |
| `run-apptests.sh`, `apptest.sh` | starts Zed, Blender, Kdenlive, darktable, `vkcube` and `glxgears` in a session, GPU on and off |
| `cleanup.sh`, `cleanup-remote.sh` | stop the clients and workloads, remove the `lt*` accounts |

**Things that went wrong, so that they do not again.**
- `pkill -f PATTERN` over SSH also kills the remote shell if its own command line contains PATTERN (SSH ends with 255): write `[s]dl-freerdp3`, or use `pkill -x`.
- `intel_gpu_top -J` must write to a **file**, not a pipe: with many clients one sample is large, the pipe fills and the tool blocks.
- LibreOffice rewrites a pre-seeded `registrymodifications.xcu` at its first start, so its "Tip of the Day" is closed by `swaymsg '[title="Tip of the Day.*"] kill'` instead; GIMP's welcome window ignores Escape, `kill` closes it.
- The windows must be started through `swaymsg exec` (they then inherit sway's `DISPLAY`), not from the shell of `runuser`: Blender and `glxgears` need X11.
- A laptop that sleeps ends the run: `caffeinate -dis` on macOS, plugged in.
- `waydesk-admin status --json --watch 1` follows a machine under load without these scripts; `sample.py` predates it.
