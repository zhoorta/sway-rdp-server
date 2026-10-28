# GPU support: plan

Status: stages 1 and 2 committed; stage 3 (zero copy) built and measured but NOT committed, with one open bug (the "veil" in Windows App, below). 2026-10-27.

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

## Results of stage 2 (2026-10-27)

The VAAPI encoder (`src/encode_va.rs`, `src/encoder.rs`) uses `cros-codecs` 0.0.6 (BSD-3) on top of `cros-libva`. The colour conversion (BGRA to NV12) is on the CPU,
straight into the VA surface; the picture is still copied out of the compositor's shm buffer. Same i7-7700, full-screen WebGL shader at 61 fps: the session
worker takes 21 % of a core (OpenH264: 107 %). The fixed-function encoder (`EncSliceLP`) needs the GuC firmware on Skylake to Coffee Lake and fails without it,
so the code tries it first and falls back to `EncSlice`, which works.

What did not work, and why the encoder looks the way it does:
- **Hardware CBR.** Intel pads every frame to the target: a static grey picture costs 29.8 KB a frame at 8 Mbit/s (VBR: 0.08 KB). A window being dragged
  cost 5.1 Mbit/s against 1.0 for OpenH264. The chip also expects a steady frame rate, and ours come when something changes.
- **VBR** is not in the crate (it offers CBR and constant quantiser); a patch means vendoring 5 MB.
- **Changing the quantiser per frame** through `tune()`: the crate builds new parameter sets (the initial QP is in the PPS) and carries on with P frames,
  with no IDR (a forced keyframe is only a non-IDR I picture there; just the first frame is an IDR). A decoder may keep the old PPS. So a new
  quantiser makes a new encoder now (`submit` in `encode_va.rs`): its first frame is a real IDR with fresh SPS and PPS. `BackendRequest` has private fields, so its lower layer (per-frame `slice_qp_delta`) cannot be driven from outside.
- So: constant quantiser, two steps below `--quality` (18 by default), and a controller (`QpControl`) that moves it up in tiers to 40 only after a second of
  overload and back after five of calm, never twice in two seconds. Dragging a window: 1.0 to 1.3 Mbit/s (OpenH264: 1.0). A random-noise shader cannot reach
  8 Mbit/s at any sensible quantiser, with either encoder.

Testing on a real link showed what the slowness of the first runs was: an SSH tunnel to Germany that delivered 0.7 Mbit/s (the daemon's log line
`the encoder's bitrate follows what the link delivers` said so). On a link with 50 Mbit/s down the same session, with the browser, the text page, a dragged
window and the WebGL page, was smooth, "even the OpenGL".

Still open: stage 3 (dmabuf capture and the colour conversion on the GPU, which would take the CPU copy and `bgra_to_yuv_nv12` away); text sharpness of the
hardware encoder against OpenH264's screen-content mode (looked the same by eye, no PSNR yet); AMD (`mesa-va-drivers`) and NVIDIA (NVENC, not VAAPI) are untested;
many users at once on one GPU; the arm64 build with `vaapi` (compiles in the build container, no GPU to run it on).

## Stage 3: zero copy (2026-10-27)

The compositor renders the capture into dmabufs (GBM, linear XRGB8888, `src/dmabuf.rs`); the encoder imports one as a VA surface and the GPU's video
processor converts it to NV12 (`src/vpp.rs`), so the picture never reaches the CPU. The VA binding needed two things it does not offer: the display
handle and the video-processing structures, so `cros-libva` is vendored with a five-line patch (`patches/cros-libva.diff`).

- `copy_with_damage` writes only the changed regions, so rotating buffers come out mostly black. The capture uses one persistent *oracle* buffer for the
  damage and a plain `copy` into rotating snapshot buffers for the picture (two requests per frame; a lease keeps a buffer from being reused while the
  encoder holds it). The damage is trusted (never short in drag tests at scale 1, 1.5 and 2), with a full refresh every 10 s as a safety net.
- Session worker CPU for the full-screen WebGL page at 20 fps: about 2 to 3 % of a core, against about 21 % with the shm capture and the CPU conversion.
- The video processor's default chroma siting is *left*; the CPU conversion averages at the *centre*. Thin light strokes on dark (text) came out with colour
  fringes (chroma error 5.2 against the CPU's); centre siting gives 0.08 and the top-bar chroma went from 39 to 50 dB. The scaling quality flags change nothing.
- A bug of ours: `DmaPool` dropped its GBM device before the buffer objects (a segfault whenever a session ended).
- `WAYDESK_DUMP_DIR` writes every frame and its bitstream (`src-NNNNN.bgra`, `stream.h264`, `frames.txt`, `rects.txt`) for decoding and comparing (PSNR, and a
  simulation of a client that paints only the dirty rectangles). Off by default: it writes the screen to disk.

### The veil in Windows App (2026-10-28, fixed)

With the VAAPI encoder, while the full-screen WebGL page was open, Windows App (Mac) showed the whole screen washed out: nothing darker than about RGB 100,
nothing brighter than about 190, also on areas that never changed, with ghost text and teal streaks at window edges, until the page was closed and a window
dragged. The software encoder was clean, and our stream decoded correctly in ffmpeg every time.

The cause was the quantiser being in the PPS (`pic_init_qp`): each tier change (18, 25, 32, 40 within a few seconds under noise) meant new parameter sets, first
through `tune()` and then through a new encoder with a fresh IDR. Several of those in a row and the client was left decoding against grey. The bisect, all with
VAAPI and the shm capture (so not the zero-copy path, and not the High profile or the missing `bitstream_restriction` either; we write that now, as OpenH264
does, and it changed nothing): quantiser pinned at 26, clean; 26 to 27 (one new encoder), clean; 26 up to 32 (three in 5 s), veil; pinned at 32, clean. So not
the value of the quantiser and not one new sequence, but parameter sets that keep changing.

The fix is in the vendored `cros-codecs` (`patches/cros-codecs.diff`): the PPS keeps `pic_init_qp` 26 for good, each slice carries the real quantiser in
`slice_qp_delta`, and `tune()` only makes new parameter sets when the frame rate changes. A quantiser change is then just a different number in the next slice
header: no new encoder, no IDR. One IDR in a whole run of the harness, slice deltas of -8, -1, 6 and 14, no veil in Windows App with the normal tiers.

Also in that patch: with constant quality the coded buffer was a fixed 1.5 MB, and a keyframe of full-screen noise at QP 18 (1.8 MB) overflowed it ("allocated
memory size is not enough"), which ended the session's connection, again at every reconnect while the noise was on screen. The buffer is now as large as a raw
NV12 picture. Not yet tried: zero-copy with the fix in Windows App, and the same in sdl-freerdp.

## NVIDIA, a first look (2026-10-28)

Server: Hetzner auction, i5-13500 (Intel UHD 770) and an RTX 4000 SFF Ada, 62 GB, Debian 13, kernel 6.12, installed with `installimage` and kernel mode
setting on. Two render nodes: `renderD128` the Intel iGPU, `renderD129` the NVIDIA card.

- **Debian's driver (550.163) does not work for a headless sway.** On `renderD129` the GLES2 renderer starts, but the output buffers cannot be allocated
  (`gbm_bo_create failed: Permission denied`, `DRM_IOCTL_MODE_CREATE_DUMB failed`); the Vulkan renderer starts and fails to allocate too. Opening the card node
  to the group changes nothing, and wlroots refuses a card node for `WLR_RENDER_DRM_DEVICE`.
- **NVIDIA's own 615.71.10 does** (`nvidia-open` from `developer.download.nvidia.com/compute/cuda/repos/debian13`): both renderers allocate with no errors.
- **A session then runs on the NVIDIA GPU.** Sway renders on `renderD129`; a full-screen WebGL page in Chromium shows 52 fps with the card 41 % busy. Things it needed:
  the GPU rule of `packaging/udev/70-waydesk-gpu.rules` (the render node opened to `waydesk-users`), the GPU-aware `/etc/chromium.d/waydesk` of this repository
  (the one of 0.3.1 adds `--disable-gpu` unconditionally), and the capture to take 24-bit pixels: NVIDIA's GLES reads back `Bgr888`, which ended the session
  until 6dfb97c.
- **The picture still goes through the CPU.** The GBM buffer of the zero-copy capture (linear XRGB) is refused by NVIDIA's allocator (`EINVAL`), so the capture falls back to
  shared memory, and VAAPI has no encoder on NVIDIA: `VAAPI encoder not available`, OpenH264 on the CPU (about 13 % of a core for one full-screen WebGL page at 52 fps).
- **NVENC works.** `ffmpeg h264_nvenc` encodes a 1680x1050 test pattern at 527 fps; the Intel iGPU of the same box through VAAPI: 446 fps; x264 ultrafast: 1012 fps on
  20 threads (speed is not what matters, the CPU it takes is).
- **A machine with both GPUs already works**: sway on the Intel node (`WAYDESK_GPU=/dev/dri/renderD128`), GLES2 and Vulkan, no errors.

### NVENC (`src/encode_nv.rs`, same day)

The encoder takes the BGRA picture as it is (NVENC reads ARGB, so no colour conversion on our side): each frame goes into a CUDA buffer and is encoded
at a constant quantiser, which `QpControl` (now `src/qp.rs`, shared with the VAAPI encoder) moves with the link. A quantiser change goes to the running encoder
(`nvEncReconfigureEncoder`) and shows up as a different `slice_qp_delta`, not as new parameter sets. `WAYDESK_ENCODER=auto` tries VAAPI, then NVENC, then
OpenH264; `nvenc` forces it. The libraries (`libnvidia-encode`, `libcuda`) are loaded when an encoder is created, so the binary starts on a machine without NVIDIA's driver.

- **Bindings:** `nvidia-video-codec-sdk` 0.4 (MIT), vendored with a small patch (`patches/nvidia-video-codec-sdk.diff`): it loads `libnvidia-encode` at run time instead of linking it,
  needs no CUDA toolkit to build, and gives access to the encoder handle and the picture flags.
- **Things it taught:** CUDA memory from the stream-ordered pool (what cudarc allocates on devices that have one) cannot be registered as an NVENC input, plain
  `cuMemAlloc` can; the settle refresh has to be an intra picture (`FORCEINTRA`), not an IDR, or every refresh sends new SPS and PPS.
- **Measured** (RTX 4000 SFF Ada, driver 615, one user, full-screen WebGL noise at 61 fps): encoder 4 to 9 ms a frame, the card's encoder engine 4 % busy, the session
  worker 8 % of a core against 13 % with OpenH264 (what is left is the shared-memory capture and the copy to the card). The stream decodes without errors in ffmpeg
  (High profile, CABAC, one reference frame), three quantiser changes in a session made no new parameter sets.

Not done: zero-copy from NVIDIA (the dmabuf to NVENC), GL and Vulkan apps measured on it, several users on one card, the stream in Windows App and the Windows client
(this one is CABAC, the VAAPI one CAVLC).
