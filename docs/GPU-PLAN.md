# GPU support: plan

Status: stage 1 built and measured (uncommitted), stages 2 and 3 open. 2026-10-27.

## Today

- Sway is forced to software: `WLR_BACKENDS=headless`, `WLR_RENDERER=pixman` (`src/worker.rs`). The shell and apps render with Mesa llvmpipe.
- Capture is `wlr-screencopy` into a `wl_shm` buffer, copied row by row into a `Vec` (`src/capture.rs`).
- Encoding is OpenH264 on the CPU (`src/encode.rs`: BGRA to YUV420 on the CPU, then OpenH264), sent over EGFX (`src/egfx.rs`). The RDP side already
  speaks H.264, so a hardware encoder only replaces the encoder, not the protocol.
- No GPU on the dev machines (`/dev/dri` is absent), so everything here needs test hardware.

## Principle

Detect and use, never require. No render node, or any step failing: today's behaviour, unchanged. An override in `/etc/default/waydesk`
(`WAYDESK_GPU=auto|off|<renderD node>`), `auto` by default.

## Stages (each useful on its own)

1. **Render on the GPU.**
   - At sway start: if a render node exists, `WLR_RENDERER=gles2` (or `vulkan`) and `WLR_RENDER_DRM_DEVICE=/dev/dri/renderD128`; else pixman.
   - Session users need access to the node (`render` group, or a udev ACL); the helper adds it only when a GPU is detected.
   - Apps: Mesa picks the render node itself; Chromium gets hardware WebGL, canvas and video decode (VAAPI flags). The DMS shell stops using llvmpipe
     (about 400 MB resident and a good part of the idle CPU per session).
   - Check: `WLR_RENDERER=gles2` with the headless backend on a real node, many outputs created and destroyed (our sessions do this).
   - Risk: the NVIDIA proprietary driver with wlroots (GBM needed); start with Intel/AMD and test NVIDIA separately.
2. **Hardware H.264 encode.** Add an `Encoder` trait with OpenH264 and a VAAPI backend (NVENC after). Same output contract as `H264::encode`: Annex B,
   SPS/PPS extraction, forced IDR, runtime bitrate. Pick at runtime, fall back to OpenH264 if the device or profile fails. Input is still the shm frame
   uploaded to the GPU, so the CPU saves the YUV conversion and the encode, not the capture copy.
3. **Zero copy.** Capture as dmabuf (`zwlr_screencopy` dmabuf events, or `ext-image-copy-capture`, which sway offers), convert BGRA to NV12 on the GPU and hand
   the surface to the encoder; frames never touch the CPU. Biggest change (the worker/session process split passes frames over `fdpass.rs`), so last, and
   only if stage 2 measurements show the copy matters.

## Open questions to settle first

- Which hardware to test on. Cheapest start: a server with an Intel or AMD iGPU or a small GPU (Hetzner dedicated GPU line) rented by the hour.
  VAAPI on Intel/AMD is the easy path; NVIDIA needs the proprietary driver.
- Screen content: do hardware encoders match OpenH264's screen-content mode on text sharpness? Measure PSNR/legibility as in `docs/MEASUREMENTS.md`.
- Per-user GPU limits: render nodes are shared, there is no per-account GPU quota. Document it; GPU memory isolation is a driver matter.
- A virtio-gpu path (QEMU dev VM) is only for testing stage 1 with the host GPU; it will not be a production target.

## How to measure

Same method as `docs/MEASUREMENTS.md`: N heavy users, CPU per session and in total, RSS per session, encode time per frame, bitrate. Software against
GPU on the same server where possible. Success for stage 1: lower CPU and memory per session; stage 2: encode time per frame well under the 8 ms
(720p) of OpenH264 screen mode, and the same or better text legibility.

## Results of stage 1 (2026-10-27)

Server: Hetzner auction, i7-7700 (4 cores / 8 threads, HD Graphics 630, Quick Sync), 62 GB, Debian 13, kernel 6.12, Mesa 25.x, one session, 1680x1050 client
(sdl-freerdp on the server itself). Installed with `installimage`: Hetzner's image adds `nomodeset` to the kernel command line and blacklists `i915`
(`/etc/modprobe.d/blacklist-hetzner.conf`); both have to go before `/dev/dri` appears.

| | software (0.3.1: pixman + llvmpipe) | GPU (`WAYDESK_GPU=auto`) |
|---|---|---|
| sway renderer | pixman | gles2 on `renderD128` |
| glmark2 (terrain, jellyfish, shading) score | 167 | **2840** |
| CPU of the account while it runs | about 2.6 cores | about 0.5 core |
| Chromium WebGL page (full-screen shader) | does not animate (`--disable-gpu`) | 62 fps |

What it needed beyond the renderer switch:
- The GLES renderer gives screencopy frames as `Xbgr8888`; `capture.rs` only took `Xrgb8888`, so the first GPU session ended at once. It now swaps R and B.
- Sessions run as `waydesk-session@UID.service`, not through `spawn_worker`, so the node is picked by the session itself and opened to the group
  `waydesk-users` with an ACL from a udev rule (`packaging/udev/70-waydesk-gpu.rules`), which also covers the account's own user services (the shell, apps).
- Chromium's `--disable-gpu` (`/etc/chromium.d/waydesk`, and the Flathub branch of `waydesk-shell`) now only applies without a usable render node.

With a full-screen 60 fps animation the account's CPU is dominated by our own encoder, not by rendering: the session worker (capture, BGRA to YUV, OpenH264)
used 107 % of a core, Chromium 8 %, its GPU process 6 %, sway 4 %. Stage 2 (hardware encode) is where the rest of the saving is.
