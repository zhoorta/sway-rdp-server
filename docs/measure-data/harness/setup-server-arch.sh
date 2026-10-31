#!/bin/bash
# setup-server-arch.sh : on the machine under test (Arch, as root), once. Installs what the workload needs and creates /srv/load, /srv/capacity, /root/load.
# Run it from this folder. It does not run -Syu: do the system update yourself BEFORE the clean-baseline reboot, not after, so the runs start from one state.
set -e
pacman -S --needed --noconfirm flatpak gimp intel-gpu-tools wtype foot chromium python imagemagick ffmpeg libreoffice-fresh
flatpak remote-add --system --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo
flatpak install --system -y flathub com.visualstudio.code
mkdir -p /srv/load /root/load /srv/capacity
cp page.html video.html video-audio.html gl.html term.sh work.sh sample.py /srv/load/ && chmod 755 /srv/load/*.sh /srv/load/sample.py
cp cleanup.sh fps.sh apptest.sh /root/load/ && chmod 755 /root/load/*.sh
python3 -c 'import random; random.seed(7); print(",".join("col%d"%i for i in range(10))); [print(",".join(str(round(random.uniform(0,10000),2)) for _ in range(10))) for _ in range(300000)]' > /srv/capacity/data.csv
(cd /srv/capacity && soffice --headless --convert-to ods data.csv)   # Calc opens the .ods without the import dialog
# a source tree of about 30 MB for VS Code: the standard library without site-packages (Debian 13's python3.13 was 28 MB; Arch's whole /usr/lib/python3.14 is 232 MB with site-packages)
rm -rf /srv/capacity/tree && mkdir /srv/capacity/tree && (cd "$(ls -d /usr/lib/python3.* | head -1)" && tar cf - --exclude=site-packages --exclude=__pycache__ .) | tar xf - -C /srv/capacity/tree
convert -size 4000x3000 plasma:fractal -blur 0x1 /srv/load/img.jpg
ffmpeg -y -f lavfi -i testsrc2=size=1280x720:rate=30 -t 10 -c:v libx264 -pix_fmt yuv420p /srv/load/clip.mp4
ffmpeg -y -f lavfi -i testsrc2=size=1280x720:rate=30 -f lavfi -i "aevalsrc=0.2*sin(2*PI*440*t)|0.2*sin(2*PI*554*t):s=48000" -t 10 -c:v libx264 -pix_fmt yuv420p -c:a aac /srv/load/clip-audio.mp4
chmod -R a+rX /srv/load /srv/capacity
du -sh /srv/capacity/tree; ls -la /srv/load /srv/capacity
