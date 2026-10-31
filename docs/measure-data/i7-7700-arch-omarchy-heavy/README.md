# Omarchy (Hyprland) heavy ramp, 2026-10-31

`../i7-7700-arch-omarchy-heavy.tsv` and `step-N.json` (`waydesk-admin status --json` after each step), made by `harness/ramp-desktops.sh 40 omarchy-heavy omarchy heavy`.
i7-7700 (4 cores / 8 threads, HD 630), 62 GB, Arch + Omarchy 4.0.6, `waydesk-server` 0.4.0-2, card-node udev rule installed; clients on a Hetzner cloud server (8 dedicated vCPU, same data centre, 1.7 ms).
Stopped at user 9 on CPU (92.2 % server side; criterion 90 %). At that point 37.9 GB of memory were still available, the iGPU render engine was 87 % busy and the video (encode) engine 38 %.

**Not a fair measure of the Hyprland backend yet.** Every Hyprland session logged "the compositor offers a dmabuf format the capture does not take: shm" (Hyprland offers ARGB8888, `src/capture.rs` takes only XRGB8888), so all sessions used the CPU copy path:
the session process used about 18 % of a core for a mostly still session and about 45 % for a video one, against about 2 % per user on sway with the GPU in the 10-28 run. Rerun after the capture accepts ARGB8888.
