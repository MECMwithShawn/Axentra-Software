#!/usr/bin/env bash
# Explicit image build stage, also bundled by the desktop preparation worker.
set -Eeuo pipefail
if [[ ${1:-} != --inside-private-mounts ]]; then
    exec unshare --mount --propagation private -- "$0" --inside-private-mounts "$@"
fi
shift
[[ $EUID == 0 && $# == 3 ]] || { echo "Usage: sudo $0 trusted-zorin.iso output.iso NEW-work-directory" >&2; exit 1; }
source_iso=$(realpath "$1")
output=$(realpath -m "$2")
work=$(realpath -m "$3")
here=$(cd -- "$(dirname -- "$0")" && pwd)
[[ -f $source_iso && ! -e $output && ! -e $work && $output != /dev/* && $work != /dev/* && $output != "$work/"* ]] || exit 1
for tool in xorriso unsquashfs mksquashfs unmkinitramfs lsinitramfs chroot mount python3; do command -v "$tool" >/dev/null; done
mkdir -p "$work"
root=$work/root
mounted=()
cleanup() {
    local d failed=0
    for ((d=${#mounted[@]}-1; d>=0; d--)); do umount "${mounted[d]}" || failed=1; done
    return "$failed"
}
trap 'cleanup || true' EXIT
echo "AXENTRA: Extracting the source ISO" >&2
xorriso -osirrox on -indev "$source_iso" -extract / "$work/iso"
iso=$work/iso
[[ $(find "$iso/casper" -maxdepth 1 -name '*.squashfs' | wc -l) == 1 ]]
grep -qi zorin "$iso/.disk/info"
[[ -f $iso/casper/initrd.zstd && -f $iso/casper/vmlinuz ]]
[[ -f $iso/EFI/BOOT/BOOTX64.EFI || -f $iso/efi/boot/bootx64.efi ]]
# Fail closed on layered or unfamiliar layouts; no partition/loop device needed.
echo "AXENTRA: Expanding the live filesystem" >&2
unsquashfs -d "$root" "$iso/casper/filesystem.squashfs"
grep -Eq '^ID="?zorin"?$' "$root/etc/os-release"
grep -Eq '^VERSION_ID="?18([."][0-9.]*)?"?$' "$root/etc/os-release"
[[ -x $root/usr/sbin/update-initramfs && -d $root/usr/share/initramfs-tools/scripts/casper-bottom ]]
unmkinitramfs "$iso/casper/initrd.zstd" "$work/original-initrd"
original=$work/original-initrd
[[ ! -d $original/main ]] || original=$original/main
[[ -f $original/conf/uuid.conf ]]
# Match the actual live kernel; never regenerate for the build host's kernel.
kernel=
for v in "$root"/boot/vmlinuz-*; do
    if cmp -s "$v" "$iso/casper/vmlinuz"; then kernel=${v##*/vmlinuz-}; break; fi
done
[[ -n $kernel && -d $root/lib/modules/$kernel ]] || { echo 'No matching live kernel/modules' >&2; exit 1; }
# Do not expose host block devices or writable firmware variables to package scripts.
mkdir -p "$root"/{dev,proc,sys,run}
mount -t tmpfs tmpfs "$root/dev"; mounted+=("$root/dev")
for dev in null zero random urandom; do touch "$root/dev/$dev"; mount --bind "/dev/$dev" "$root/dev/$dev"; mounted+=("$root/dev/$dev"); done
mount -t proc proc "$root/proc"; mounted+=("$root/proc")
# policy-rc.d forbids daemon starts during package installation.
[[ ! -e $root/usr/sbin/policy-rc.d ]] || { echo 'Existing policy-rc.d; unsupported image' >&2; exit 1; }
printf '#!/bin/sh\nexit 101\n' > "$root/usr/sbin/policy-rc.d"
chmod +x "$root/usr/sbin/policy-rc.d"
# Uses the trusted image's configured repositories; retain apt logs/package versions.
# Resolve repository hosts inside the chroot, then restore the live image's resolver.
cp -a "$root/etc/resolv.conf" "$work/resolv.conf.original"
rm "$root/etc/resolv.conf"
cp -L /etc/resolv.conf "$root/etc/resolv.conf"
echo "AXENTRA: Installing mdadm and mdmon inside the image" >&2
chroot "$root" apt-get update
chroot "$root" env DEBIAN_FRONTEND=noninteractive apt-get install -y mdadm initramfs-tools python3 python3-tk xterm
install -Dm755 "$here/zz-axentra-imsm" "$root/etc/initramfs-tools/hooks/zz-axentra-imsm"
install -Dm755 "$here/axentra-imsm" "$root/etc/initramfs-tools/scripts/local-top/axentra-imsm"
install -Dm755 "$here/axentra-imsm" "$root/etc/initramfs-tools/scripts/casper-premount/axentra-imsm"
install -Dm755 "$here/axentra_raid.py" "$root/usr/local/sbin/axentra-raid-finalize"
install -Dm755 "$here/assistant.py" "$root/usr/local/bin/axentra-install-assistant"
chroot "$root" python3 -c 'import tkinter'
printf 'DEVICE partitions containers\nAUTO +imsm -all\nHOMEHOST <ignore>\n' > "$root/etc/mdadm/mdadm.conf"

# Keep the target-finalization instructions available offline on live media.
install -Dm644 "$here/INSTALL-RAID.txt" "$root/usr/share/doc/axentra/INSTALL-RAID.txt"
mkdir -p "$root/usr/share/applications"
cat > "$root/usr/share/applications/axentra-raid-instructions.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=Axentra RAID installation instructions
Comment=Read before installing or restarting on Intel IMSM RAID
Exec=xdg-open /usr/share/doc/axentra/INSTALL-RAID.txt
Icon=drive-harddisk
Terminal=false
Categories=System;
DESKTOP
cat > "$root/usr/share/applications/axentra-install-assistant.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=Axentra Installation Assistant
Comment=Review RAID partitions and finalize installed Zorin before reboot
Exec=/usr/local/bin/axentra-install-assistant
Icon=drive-harddisk
Terminal=false
Categories=System;
DESKTOP

# Preserve Casper's live-media UUID and boot mode without making target kernels live kernels.
install -Dm644 "$original/conf/uuid.conf" "$root/etc/axentra-live-uuid"
cat > "$root/etc/initramfs-tools/hooks/zzz-axentra-live" <<'HOOK'
#!/bin/sh
set -eu
case "${1:-}" in prereqs) echo zz-axentra-imsm; exit 0;; esac
mkdir -p "$DESTDIR/conf/conf.d"
cp /etc/axentra-live-uuid "$DESTDIR/conf/uuid.conf"
printf 'BOOT=casper\n' > "$DESTDIR/conf/conf.d/default-boot-to-casper.conf"
HOOK
chmod +x "$root/etc/initramfs-tools/hooks/zzz-axentra-live"
echo "AXENTRA: Rebuilding and checking the live initramfs" >&2
chroot "$root" mkinitramfs -c gzip -o /tmp/axentra-live-initrd "$kernel"
lsinitramfs "$root/tmp/axentra-live-initrd" > "$work/initrd-files.txt"
python3 - "$here" "$work/initrd-files.txt" <<'PY'
import sys
sys.path.insert(0, sys.argv[1])
from axentra_raid import verify_initrd
listing = open(sys.argv[2]).read()
verify_initrd(listing)
assert 'scripts/casper-premount/axentra-imsm' in listing, 'Missing live Casper assembly hook'
PY
unmkinitramfs "$root/tmp/axentra-live-initrd" "$work/verified-initrd"
verified=$work/verified-initrd
[[ ! -d $verified/main ]] || verified=$verified/main
cmp "$original/conf/uuid.conf" "$verified/conf/uuid.conf"
cmp "$root/etc/mdadm/mdadm.conf" "$verified/etc/mdadm/mdadm.conf"
cp "$root/tmp/axentra-live-initrd" "$iso/casper/initrd.zstd"
rm "$root/tmp/axentra-live-initrd" "$root/etc/axentra-live-uuid" "$root/etc/initramfs-tools/hooks/zzz-axentra-live" "$root/usr/sbin/policy-rc.d"
rm "$root/etc/resolv.conf"
cp -a "$work/resolv.conf.original" "$root/etc/resolv.conf"
# dpkg-query expands these placeholders, not the build shell.
# shellcheck disable=SC2016
chroot "$root" dpkg-query -W '-f=${Package} ${Version}\n' > "$iso/casper/filesystem.manifest"
# Ubiquity must retain mdadm in the installed target.
for manifest in "$iso/casper/filesystem.manifest-remove" "$iso/casper/filesystem.manifest-desktop"; do
    if [[ -f $manifest ]]; then
        if [[ $manifest == *-remove ]]; then sed -i '/^mdadm\([[:space:]]\|$\)/d' "$manifest"; else
            sed -i '/^mdadm[[:space:]]/d' "$manifest"
            grep '^mdadm ' "$iso/casper/filesystem.manifest" >> "$manifest"
        fi
    fi
done
cleanup
mounted=()
echo "AXENTRA: Compressing the prepared live filesystem" >&2
mksquashfs "$root" "$work/filesystem.squashfs" -noappend -comp xz
mv "$work/filesystem.squashfs" "$iso/casper/filesystem.squashfs"
du -sx --block-size=1 "$root" | cut -f1 > "$iso/casper/filesystem.size"
# Recompute media checksums; preserve original EFI executables and boot catalog.
(cd "$iso" && find . -type f ! -name md5sum.txt ! -path './boot.catalog' -print0 | sort -z | xargs -0 md5sum > "$work/md5sum.txt")
mv "$work/md5sum.txt" "$iso/md5sum.txt"
echo "AXENTRA: Rebuilding the ISO and preserving its boot equipment" >&2
xorriso -indev "$source_iso" -outdev "$output" -boot_image any replay \
    -map "$iso/casper" /casper -map "$iso/md5sum.txt" /md5sum.txt -commit
sha256sum "$source_iso" "$output" | tee "$work/image-sha256.txt"
echo 'ISO built. Use Axentra Original image mode; validate a spare USB and target before production use.'
