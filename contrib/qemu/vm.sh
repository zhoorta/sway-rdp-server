#!/bin/sh
# The development VM of sway-rdp: Debian 13 arm64 from the official cloud image, run directly with QEMU (hypervisor.framework, so
# at near native speed on Apple Silicon), headless. Set up on first boot by cloud-init (user-data.in: a user with the key of this
# script, sudo, the guest agent).
#
#   contrib/qemu/vm.sh create      makes the disk and the seed, starts the VM, waits until ssh answers
#   contrib/qemu/vm.sh start|stop  stop sends an ACPI power button, and kills the VM when it does not shut down in 30 s
#   contrib/qemu/vm.sh status
#   contrib/qemu/vm.sh ssh [cmd]   ssh into it as jose (key and port come from here)
#   contrib/qemu/vm.sh console     the serial console (leave with Ctrl-C); the whole boot log is in $DIR/console.log
#   contrib/qemu/vm.sh destroy     stops it and deletes the VM and its disk
#
# Needs QEMU (brew install qemu). The cloud image is downloaded to ~/vm-images when it is not there yet.
# Ports on the Mac (loopback only): 2222 -> ssh, 3395 -> RDP (the RDP server of the VM has to listen on 0.0.0.0).
# Variables: VM_NAME (sway-rdp-dev), VM_DIR (~/vm/$VM_NAME), CPUS (4), RAM in MiB (4096), DISK (30G).
set -eu
NAME=${VM_NAME:-sway-rdp-dev}
DIR=${VM_DIR:-$HOME/vm/$NAME}
IMAGES="$HOME/vm-images"
IMAGE="$IMAGES/debian-13-genericcloud-arm64.qcow2"
KEY="$HOME/.ssh/id_ed25519_sway_rdp_vm"
HERE=$(cd "$(dirname "$0")" && pwd)
CPUS=${CPUS:-4}
RAM=${RAM:-4096}
DISK=${DISK:-30G}
QEMU_SHARE=$(brew --prefix qemu 2>/dev/null)/share/qemu

running() { [ -f "$DIR/qemu.pid" ] && kill -0 "$(cat "$DIR/qemu.pid")" 2>/dev/null; }

vm_ssh() {
    ssh -q -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=5 -o BatchMode=yes -o "SendEnv -*" \
        -i "$KEY" -p 2222 jose@127.0.0.1 "$@"
}

wait_ssh() {
    echo "waiting for ssh on 127.0.0.1:2222 ..."
    for _ in $(seq 1 90); do
        if vm_ssh true 2>/dev/null; then echo "up: contrib/qemu/vm.sh ssh"; return 0; fi
        running || { echo "the VM stopped: see $DIR/console.log" >&2; return 1; }
        sleep 4
    done
    echo "no ssh after 6 minutes: see $DIR/console.log" >&2
    return 1
}

start() {
    running && { echo "already running (pid $(cat "$DIR/qemu.pid"))"; return 0; }
    [ -f "$DIR/disk.qcow2" ] || { echo "no VM here yet: run create" >&2; return 1; }
    rm -f "$DIR/console.sock" "$DIR/monitor.sock"
    qemu-system-aarch64 -name "$NAME" -M virt,gic-version=3 -accel hvf -cpu host -smp "$CPUS" -m "$RAM" \
        -drive if=pflash,format=raw,file="$QEMU_SHARE/edk2-aarch64-code.fd",readonly=on \
        -drive if=pflash,format=raw,file="$DIR/vars.fd" \
        -drive file="$DIR/disk.qcow2",if=virtio,format=qcow2 \
        -drive file="$DIR/seed.iso",if=virtio,format=raw,media=cdrom,readonly=on \
        -netdev user,id=n0,hostfwd=tcp:127.0.0.1:2222-:22,hostfwd=tcp:127.0.0.1:3395-:3389 -device virtio-net-pci,netdev=n0 \
        -device virtio-rng-pci \
        -chardev socket,id=ser0,path="$DIR/console.sock",server=on,wait=off,logfile="$DIR/console.log" -serial chardev:ser0 \
        -monitor unix:"$DIR/monitor.sock",server,nowait \
        -display none -daemonize -pidfile "$DIR/qemu.pid"
}

stop() {
    running || { echo "not running"; return 0; }
    pid=$(cat "$DIR/qemu.pid")
    printf 'system_powerdown\n' | nc -U -w 2 "$DIR/monitor.sock" >/dev/null 2>&1 || true
    for _ in $(seq 1 30); do kill -0 "$pid" 2>/dev/null || { echo "stopped"; return 0; }; sleep 1; done
    echo "no shutdown in 30 s: killing it" >&2
    kill "$pid" 2>/dev/null || true
}

create() {
    command -v qemu-system-aarch64 >/dev/null || { echo "QEMU is missing: brew install qemu" >&2; exit 1; }
    [ -e "$DIR/disk.qcow2" ] && { echo "$DIR already has a VM: destroy it first" >&2; exit 1; }
    mkdir -p "$IMAGES" "$DIR"
    if [ ! -f "$IMAGE" ]; then
        curl -fL -o "$IMAGE" https://cloud.debian.org/images/cloud/trixie/latest/debian-13-genericcloud-arm64.qcow2
    fi
    [ -f "$KEY" ] || ssh-keygen -t ed25519 -N '' -C "sway-rdp-vm" -f "$KEY" >/dev/null
    # cloud-init (NoCloud): a seed ISO with the label "cidata".
    mkdir -p "$DIR/seed"
    sed -e "s|@PUBKEY@|$(cat "$KEY.pub")|" -e "s|@HOSTNAME@|$NAME|" "$HERE/user-data.in" > "$DIR/seed/user-data"
    printf 'instance-id: %s-%s\nlocal-hostname: %s\n' "$NAME" "$(date +%s)" "$NAME" > "$DIR/seed/meta-data"
    hdiutil makehybrid -quiet -iso -joliet -default-volume-name cidata -o "$DIR/seed.iso" "$DIR/seed"
    rm -rf "$DIR/seed"
    cp "$IMAGE" "$DIR/disk.qcow2"
    qemu-img resize -q "$DIR/disk.qcow2" "$DISK"
    dd if=/dev/zero of="$DIR/vars.fd" bs=1m count=64 2>/dev/null # the UEFI variables
    start
    wait_ssh
}

case "${1:-}" in
    create) create ;;
    start) start ;;
    stop) stop ;;
    status) if running; then echo "running (pid $(cat "$DIR/qemu.pid")), ssh: $(vm_ssh true 2>/dev/null && echo up || echo no)"; else echo "stopped"; fi ;;
    ssh) shift; vm_ssh "$@" ;;
    console) exec nc -U "$DIR/console.sock" ;;
    destroy) stop; rm -rf "$DIR"; echo "deleted $DIR" ;;
    *) sed -n '2,15p' "$0"; exit 1 ;;
esac
