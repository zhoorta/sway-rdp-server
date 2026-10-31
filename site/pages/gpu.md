title: Using a GPU
nav: GPU
seo_title: GPU acceleration for a remote Linux desktop: Intel, NVIDIA, AMD · Waydesk
description: What Waydesk does with a GPU (Intel, AMD, NVIDIA), how to set it up, how to see what each session runs, what happens when it fails, and what was measured.
---
No GPU is needed. When the machine has one, every session draws its desktop on it and encodes the H.264 picture with the chip's own encoder, which leaves the CPU to the programs, and Chromium gets WebGL. If anything about the GPU fails, the session carries on in software.

## What it changes {bar: notes}
- **The desktop** (sway, the shell, the applications' windows) is drawn on the GPU through a render node in `/dev/dri`. OpenGL and WebGL applications run on it; without a GPU they run on the CPU, and a WebGL page in the browser does not animate (see below).
- **The picture** is converted and encoded on the chip (VAAPI on Intel and AMD, NVENC on NVIDIA). On Intel the capture is also zero-copy: the compositor draws into GPU buffers and the GPU's video processor converts them, so the picture never passes through the CPU.
- **Without a GPU** OpenGL and Vulkan applications still open, drawn by Mesa's software drivers on the CPU (`glxgears`, `vkcube` and Zed opened on a server with no render node). The browser is the exception: Waydesk starts Chromium with `--disable-gpu` there, which saves memory but leaves WebGL out, so a WebGL page does not animate. `--use-angle=swiftshader --enable-unsafe-swiftshader` brings it back at about 8 fps for a full-screen page, using three of four cores. A server that has a render node while `WAYDESK_GPU=off` is a different case: `vkcube` failed there.
- **Nothing to configure** in the usual case: the package gives the accounts of `waydesk-users` the render node with a udev rule, and `WAYDESK_GPU=auto` picks the first one.

## Which GPUs {bar: table}
| GPU | state | what was done |
|---|---|---|
| Intel (HD Graphics 630) | tested | full sessions, up to 21 active heavy users, VAAPI encoder, zero-copy capture |
| Intel (UHD Graphics 770) | short test | the compositor on it with both renderers; its encoder in `ffmpeg` (446 fps at 1680x1050) |
| NVIDIA (RTX 4000 SFF Ada) | tested, **first version** | sway on the card, NVENC encoder, looked at in the Windows App and in mstsc; needs NVIDIA's own driver **615 or newer** |
| AMD | not tried | should work through VAAPI (`mesa-va-drivers`) |

## Set it up {bar: terminal}
**Intel** needs the media driver, which the package recommends: `intel-media-va-driver-non-free` (or `intel-media-va-driver`). **AMD** needs `mesa-va-drivers`. Then check that the machine has a render node and that sessions use it:

```console
$ ls /dev/dri
by-path  card0  renderD128
$ waydesk-admin status
```

On a dedicated server from an installer that adds `nomodeset` to the kernel line or blacklists the graphics driver (Hetzner's `installimage` does the first unless `USE_KERNEL_MODE_SETTING yes` is set), `/dev/dri` does not appear until that is undone.

**NVIDIA.** Debian's own driver (550) lets sway start on the card and then gives it no buffers, so a session cannot be made there (it falls back to software, see below). NVIDIA's driver 615 does. These are the steps that worked on Debian 13:

```console
$ curl -fsSLO https://developer.download.nvidia.com/compute/cuda/repos/debian13/x86_64/cuda-keyring_1.1-1_all.deb
$ sudo dpkg -i cuda-keyring_1.1-1_all.deb       # NVIDIA's apt repository and its signing key
$ sudo apt purge 'nvidia-*' 'libnvidia-*'        # Debian's 550 packages, if they are there
$ sudo apt update && sudo apt install nvidia-open
$ echo "options nvidia-drm modeset=1 fbdev=1" | sudo tee /etc/modprobe.d/nvidia-drm-modeset.conf
$ sudo update-initramfs -u && sudo reboot
$ nvidia-smi                                    # the card and the driver version
```

That adds a third-party apt repository and its key to the machine; read what you are trusting first. NVENC itself comes with the driver (`libnvidia-encode`); Waydesk loads it at run time, only when a session needs an encoder, so a machine without NVIDIA's driver is not affected.

## See what runs {bar: terminal}
`waydesk-admin status` lists the GPUs and says, for every session, whether it renders on a GPU and which encoder it runs. Two sessions on an Intel machine, as the command printed them: the first drawn on the GPU and encoded by VAAPI, the second started after `WAYDESK_GPU=off` (hence the line "new sessions use: off"), so it renders and encodes in software:

```console
$ waydesk-admin status
Memory    62.6 GB in all, 53.5 GB available
Swap      sda1 8.0 GB (0 MB used), zram0 16.0 GB (1.7 GB used)
Load      2.55 1.95 1.79 (8 cores)
GPU       Intel HD Graphics 630 (i915, renderD128): 61 % busy, 1100 MHz
          new sessions use: off
Sessions  2 running; a new one would be accepted (needs 9.4 GB available)

ACCOUNT            SESSION     MEMORY    SWAP    CPU    LIMIT RENDER ENCODER            PROGRAMS (resident)
lt1                running     860 MB       -     29%  31.3 GB gpu    vaapi q40 20fps    chromium 1066 MB, qs 329 MB, sway 104 MB
lt2                running     736 MB       -     10%  31.3 GB cpu    openh264 5fps      chromium 1109 MB, qs 399 MB, waydesk-server 77 MB
```

An NVIDIA machine (the values of two readings a minute apart, put together; the encoder engine is shown separately, and the Intel card of the same machine is idle):

```console
$ waydesk-admin status
GPU       Intel AlderLake-S GT1 (i915, renderD128): 0 % busy
          NVIDIA RTX 4000 SFF Ada Generation (nvidia, renderD129): 8 % busy, encoder 3 %, 786 MB of 20 GB, 1560 MHz, 26 W
          new sessions use: /dev/dri/renderD129

ACCOUNT            SESSION     MEMORY    SWAP    CPU    LIMIT RENDER ENCODER            PROGRAMS (resident)
lt0                running     657 MB       -     48%  31.3 GB gpu    nvenc q40 20fps    chromium 1121 MB, qs 356 MB, waydesk-server 231 MB
```

How busy a card is comes from NVIDIA's management library for NVIDIA, from `gpu_busy_percent` for AMD, and, on Intel, from the share of time the card is out of its idle state (not the split between its render and video engines). `--watch` keeps it on the screen.

## Settings {bar: terminal}
All of these go in `/etc/default/waydesk` and take effect for sessions that start afterwards (end the session or log out first).

| setting | meaning |
|---|---|
| `WAYDESK_GPU=off` | software rendering, as without a GPU. Another value: the render node to use, for example `/dev/dri/renderD129` |
| `WAYDESK_GPU_ROLES=renderD128=render,renderD129=inference` | what each card is for, by render node: `render` (sessions only), `inference` (a local model only: `auto` never picks it and no other card's session encodes on it; an account pinned there by hand still can) or `both`. Cards not listed are `render`. `waydesk-admin status` then prints the `CUDA_VISIBLE_DEVICES` to give the model service. A role starts nothing and moves nothing: you point the model service at its card yourself, and `inference` on an Intel or AMD card only keeps sessions off it |
| `WAYDESK_ENCODER=software` | keep the GPU for drawing but encode on the CPU. `auto` (the default: VAAPI, then NVENC), `vaapi` and `nvenc` are the others |
| `WAYDESK_NVENC_DEVICE=1` | the NVIDIA card (CUDA number) that encodes; by default the card the session draws on, else the first |
| `WAYDESK_DMABUF=0` | go back to copying the picture through shared memory instead of the zero-copy capture |
| `WAYDESK_VA_MIN_QP`, `WAYDESK_VA_MAX_QP` | the two ends of the quantiser (default 18 and 40): the quality wanted, and what a slow link may cost |

The GPU encoders run at a constant quantiser and move it as the link allows, one step at a time. A step is only a number in the next picture's header: no keyframe and no new parameter sets, which matters because a client can show a washed-out picture when it meets several in a row (a first version did, before this release).

## When it fails {bar: notes}
- **A session that cannot be made on the GPU is made again in software**, once, and the log says so (`the session could not be made on the GPU (...): software rendering`). Before this, Debian's NVIDIA driver left the account without a desktop.
- **An encoder that cannot start falls back to OpenH264** on the CPU: `VAAPI encoder not available (...): OpenH264 on the CPU`. The session keeps working; the worker then takes about 12 % of the machine instead of 3 % for one full-screen WebGL page.
- **GeForce cards limit simultaneous NVENC sessions** (8 on recent drivers); past the limit the new session encodes with OpenH264. Workstation and datacenter cards have no such cap. Not measured here.
- Both fallbacks show in `status`: the `RENDER` column says `cpu`, the `ENCODER` column `openh264`.

## What it measured {bar: table}
On a Core i7-7700 with its integrated HD 630, 64 GB, users added one at a time, each active (browsing, typing in a terminal, Calc on 300,000 rows, VS Code, GIMP, a video), with the RDP clients on another machine:

| active heavy users | CPU, GPU on | CPU, GPU off | capture and encoding, GPU on / off |
|---|---|---|---|
| 10 | 47 % | 45 % | 4.2 % / 8.7 % |
| 15 | 49 % | 60 % | 4.6 % / 11.7 % |
| 20 | 50 % | 69 % | 4.8 % / 15.4 % |
| 24 | refused at 22 (memory) | 94 % | – / 21.1 % |

- **The GPU flattens the CPU, but on this machine it did not add users:** 21 fit with it and 24 without, because an integrated GPU takes system memory (2.24 GB a user against 1.95 GB) and memory ended both runs.
- **Full-screen WebGL** (the heaviest picture) holds 60 fps for two users on the HD 630, 47 for three, 20 for six and 13 for ten: the card is the limit and the CPU stays under 40 %. With the GPU off the page does not animate.
- **NVIDIA:** 61 fps on the same page, 4 to 9 ms per encoded frame, the card's encoder engine 4 % busy.

The method, every table and the scripts are in [`docs/MEASUREMENTS.md`](https://github.com/zhoorta/waydesk/blob/main/docs/MEASUREMENTS.md); the notes behind the NVIDIA work in [`docs/GPU-PLAN.md`](https://github.com/zhoorta/waydesk/blob/main/docs/GPU-PLAN.md). Not measured: AMD, several users on one NVIDIA card, a card with its own memory under many users.
