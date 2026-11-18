#!/usr/bin/env python3
"""Explicit, non-partitioning IMSM target finalizer. No implicit reboot."""
import argparse
import hashlib
import os
from pathlib import Path
import re
import subprocess
import tempfile

UUID = re.compile(r"[0-9a-fA-F]{8}(?::[0-9a-fA-F]{8}){3}\Z")
BEGIN = '# BEGIN AXENTRA IMSM\n'
END = '# END AXENTRA IMSM\n'


def run(*args):
    return subprocess.check_output(args, text=True).strip()


def array_lines(scan, selected_uuid=None):
    """Keep identities, never volatile md numbers or member device paths."""
    rows = []
    containers = {}
    for line in scan.splitlines():
        words = line.split()
        if not words or words[0] != 'ARRAY':
            continue
        attrs = dict(w.split('=', 1) for w in words[1:] if '=' in w)
        uuid = attrs.get('UUID', '')
        if not UUID.fullmatch(uuid):
            raise ValueError('Missing/invalid array UUID')
        if attrs.get('metadata') == 'imsm':
            containers[words[1]] = uuid
        rows.append((attrs, uuid))
    if selected_uuid is not None:
        selected = [a for a, u in rows if u.lower() == selected_uuid.lower() and 'container' in a]
        if len(selected) != 1:
            raise ValueError('Root volume UUID missing or ambiguous in IMSM scan')
        c = selected[0]['container']
        container_uuid = containers.get(c, c)
        rows = [(a, u) for a, u in rows if u.lower() == selected_uuid.lower() or u == container_uuid]
    result = []
    for attrs, uuid in rows:
        if attrs.get('metadata') == 'imsm':
            result.append(f'ARRAY metadata=imsm UUID={uuid}')
        elif 'container' in attrs:
            container = containers.get(attrs['container'], attrs['container'])
            member = attrs.get('member', '')
            if not UUID.fullmatch(container) or container not in containers.values() or not member.isdigit():
                raise ValueError('Unresolved IMSM container/member')
            result.append(f'ARRAY /dev/md/{uuid.replace(":", "")} container={container} member={member} UUID={uuid}')
    if not containers or not any('container=' in line for line in result):
        raise ValueError('No assembled IMSM volume and container found')
    return '\n'.join(result) + '\n'


def merge_config(old, generated):
    if old.count(BEGIN) != old.count(END) or old.count(BEGIN) > 1:
        raise ValueError('Malformed managed configuration block')
    if BEGIN in old and old.index(BEGIN) > old.index(END):
        raise ValueError('Reversed managed configuration block')
    old = re.sub(re.escape(BEGIN) + '.*?' + re.escape(END), '', old, flags=re.S)
    identities = {u.lower() for u in re.findall(r'UUID=([^\s]+)', generated)}
    kept = []
    for line in old.splitlines():
        if line.strip().startswith('AUTO '):
            # Preserve policy for all other metadata while explicitly enabling IMSM.
            line = 'AUTO +imsm ' + ' '.join(x for x in line.split()[1:] if x not in ('+imsm', '-imsm'))
        if line.strip().startswith('ARRAY ') and any(u.lower() in identities for u in re.findall(r'UUID=([^\s]+)', line, re.I)):
            continue
        kept.append(line)
    if not any(x.strip().startswith('AUTO ') for x in kept):
        kept.append('AUTO +imsm -all')
    return '\n'.join(kept).rstrip() + '\n' + BEGIN + generated + END


def atomic(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(mode='w', dir=path.parent, delete=False) as f:
        f.write(text)
        f.flush()
        os.fsync(f.fileno())
    os.chmod(f.name, 0o644)
    os.replace(f.name, path)


def efi_hashes(esp):
    return {str(p.relative_to(esp)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in (esp / 'EFI').rglob('*') if p.is_file()}


def mounted(target, where):
    return run('findmnt', '-rn', '-M', str(target), '-o', where)


def finalize(target, mode):
    if os.geteuid() != 0:
        raise ValueError('Run as root from the live session')
    target = target.resolve()
    if target == Path('/') or not (target / 'etc/os-release').is_file():
        raise ValueError('Select a separately mounted installed Zorin root')
    release = (target / 'etc/os-release').read_text()
    if 'ID=zorin' not in release and 'ID="zorin"' not in release:
        raise ValueError('Only Zorin targets supported')
    source = mounted(target, 'SOURCE')
    if not source.startswith('/dev/') or mounted(target, 'FSTYPE') != 'ext4':
        raise ValueError('Only a directly mounted ext4 RAID partition is supported')
    parent = run('lsblk', '-ndo', 'PKNAME', source)
    if not re.fullmatch(r'md[0-9]+', parent):
        raise ValueError('Root must be a partition of an assembled md RAID volume')
    sysmd = Path('/sys/class/block') / parent / 'md'
    if (sysmd / 'level').read_text().strip() != 'raid0' or not (sysmd / 'metadata_version').read_text().startswith('external:/'):
        raise ValueError('Expected an IMSM RAID 0 member volume')
    container = (sysmd / 'metadata_version').read_text().strip().split('/')[1]
    if (Path('/sys/class/block') / container / 'md/metadata_version').read_text().strip() != 'external:imsm':
        raise ValueError('Root container is not IMSM')
    if not Path('/sys/firmware/efi').exists():
        raise ValueError('Boot the live USB in UEFI mode')
    esp = target / 'boot/efi'
    if mounted(esp, 'FSTYPE') != 'vfat':
        raise ValueError('Mount the existing FAT ESP at target/boot/efi without formatting')
    esp_source = mounted(esp, 'SOURCE')
    if run('lsblk', '-ndo', 'PARTTYPE', esp_source).lower() != 'c12a7328-f81f-11d2-ba4b-00a0c93ec93b':
        raise ValueError('Selected filesystem is not a GPT EFI System Partition')
    # Separate /boot must be mounted before modifying kernel files.
    fstab = run('findmnt', '--fstab', '--tab-file', str(target / 'etc/fstab'), '-rn', '-o', 'SOURCE,TARGET')
    for line in fstab.splitlines():
        spec, dest = line.split()
        if dest in ('/', '/boot', '/boot/efi'):
            if not spec.startswith('UUID='):
                raise ValueError('Root/boot/ESP fstab entries must use filesystem UUIDs')
            actual = mounted(target if dest == '/' else target / dest.lstrip('/'), 'UUID')
            if actual.lower() != spec[5:].lower():
                raise ValueError(f'fstab mismatch or missing mount: {dest}')
    if not all(any(line.endswith(' ' + d) for line in fstab.splitlines()) for d in ('/', '/boot/efi')):
        raise ValueError('Missing persistent root or ESP fstab entry')
    before = efi_hashes(esp)
    windows = 'EFI/Microsoft/Boot/bootmgfw.efi'
    if mode == 'dual' and windows not in before:
        raise ValueError('Dual boot requires the existing Microsoft EFI loader on this ESP')
    if not any(p.lower().endswith(('/shimx64.efi', '/grubx64.efi')) for p in before):
        raise ValueError('Installer must install the Linux EFI loader first')
    # Require callers to prepare chroot mounts; never guess or unmount their mounts.
    for dest in ('dev', 'proc', 'sys', 'run'):
        mounted(target / dest, 'TARGET')
    detail = dict(line.split('=', 1) for line in run('mdadm', '--detail', '--export', '/dev/' + parent).splitlines() if '=' in line)
    generated = array_lines(run('mdadm', '--detail', '--scan'), detail.get('MD_UUID', ''))
    for dest in ('/', '/boot'):
        boot_separate = any(line.endswith(' /boot') for line in fstab.splitlines())
        expected = mounted(target / 'boot' if dest == '/boot' and boot_separate else target, 'UUID')
        if run('chroot', str(target), 'grub-probe', '--target=fs_uuid', dest).lower() != expected.lower():
            raise ValueError('GRUB cannot resolve the installed root/boot filesystem UUID')
    conf = target / 'etc/mdadm/mdadm.conf'
    old = conf.read_text() if conf.exists() else ''
    new = merge_config(old, generated)
    print(new)
    phrase = 'FINALIZE ' + mounted(target, 'UUID')
    if input(f'Target {source}, ESP {esp_source}, mode {mode}. Type {phrase}: ') != phrase:
        raise ValueError('Confirmation did not match')
    # No package installation here: it can run bootloader maintainer scripts.
    if run('chroot', str(target), 'dpkg-query', '-W', '-f=${Status}', 'mdadm') != 'install ok installed':
        raise ValueError('mdadm package is not configured')
    for name in ('usr/sbin/mdadm', 'usr/sbin/mdmon'):
        if not (target / name).exists():
            raise ValueError('mdadm/mdmon missing: use the RAID-enabled installer image')
    if not (target / 'etc/initramfs-tools/hooks/zz-axentra-imsm').exists():
        raise ValueError('Axentra initramfs hook missing from target')
    if not conf.with_suffix('.conf.axentra-before').exists():
        atomic(conf.with_suffix('.conf.axentra-before'), old)
    atomic(conf, new)
    run('chroot', str(target), 'update-initramfs', '-u', '-k', 'all')
    for initrd in (target / 'boot').glob('initrd.img-*'):
        verify_initrd(run('lsinitramfs', str(initrd)))
        verify_config(initrd, new)
    if not list((target / 'boot').glob('initrd.img-*')):
        raise ValueError('No installed initramfs found')
    if mode == 'dual':
        uuid = mounted(esp, 'UUID')
        if not re.fullmatch(r'[A-Fa-f0-9]{4}-[A-Fa-f0-9]{4}', uuid):
            raise ValueError('Invalid ESP filesystem UUID')
        atomic(target / 'etc/grub.d/41_axentra_windows', '#!/bin/sh\ncat <<\'EOF\'\nmenuentry "Windows (existing EFI loader)" --class windows {\n insmod part_gpt\n insmod fat\n search --no-floppy --fs-uuid --set=root ' + uuid + '\n chainloader /EFI/Microsoft/Boot/bootmgfw.efi\n}\nEOF\n')
        os.chmod(target / 'etc/grub.d/41_axentra_windows', 0o755)
    run('chroot', str(target), 'update-grub')
    run('chroot', str(target), 'grub-script-check', '/boot/grub/grub.cfg')
    if efi_hashes(esp) != before:
        raise ValueError('EFI files changed unexpectedly. Do not reboot; inspect the ESP')
    print('Finalization complete. EFI files unchanged. Hardware boot remains unvalidated; no reboot performed.')


def verify_config(initrd, expected):
    with tempfile.TemporaryDirectory(prefix='axentra-initrd-') as tmp:
        run('unmkinitramfs', str(initrd), tmp)
        root = Path(tmp)
        if (root / 'main').is_dir():
            root = root / 'main'
        if (root / 'etc/mdadm/mdadm.conf').read_text() != expected:
            raise ValueError('Rebuilt initramfs does not contain the persistent UUID configuration')
        modules = list(root.rglob('raid0.ko*'))
        builtin = any('kernel/drivers/md/raid0.ko' in p.read_text() for p in root.rglob('modules.builtin'))
        if not modules and not builtin:
            raise ValueError('Rebuilt initramfs lacks RAID 0 driver')


def verify_initrd(listing):
    names = {line.removeprefix('./') for line in listing.splitlines()}
    for suffix in ('sbin/mdadm', 'sbin/mdmon', 'etc/mdadm/mdadm.conf', 'scripts/local-top/axentra-imsm', 'lib/udev/rules.d/64-md-raid-assembly.rules'):
        if not any(n == suffix or n == 'usr/' + suffix for n in names):
            raise ValueError('Initramfs missing ' + suffix)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', type=Path, required=True)
    parser.add_argument('--mode', choices=('dual', 'single'), required=True)
    args = parser.parse_args()
    try:
        finalize(args.target, args.mode)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f'Finalization failed; do not reboot: {error}\n')
