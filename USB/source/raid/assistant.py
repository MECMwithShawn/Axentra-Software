#!/usr/bin/env python3
"""Offline IMSM installation guide; inventory/planning never writes block devices."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

ESP = 'c12a7328-f81f-11d2-ba4b-00a0c93ec93b'
LINUX = '0fc63daf-8483-4772-8e79-3d69d8477de4'
FS_UUID = re.compile(r'(?:[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}|[0-9a-fA-F]{4}-[0-9a-fA-F]{4})\Z')
SELF = '/usr/local/bin/axentra-install-assistant'
FINALIZER = '/usr/local/sbin/axentra-raid-finalize'


def run(*args):
    return subprocess.check_output(args, text=True, timeout=30).strip()


def inventory():
    data = json.loads(run('lsblk', '--json', '--bytes', '--paths', '--output',
                         'NAME,KNAME,PKNAME,TYPE,SIZE,FSTYPE,UUID,PARTTYPE,PTTYPE,MOUNTPOINTS,RO'))
    devices = {}
    def walk(rows, parent=None):
        for row in rows:
            name = row['name']
            item = {k: v for k, v in row.items() if k != 'children'}
            item['parent'] = row.get('pkname') or parent
            if item['parent'] and not item['parent'].startswith('/dev/'):
                item['parent'] = '/dev/' + item['parent']
            devices[name] = item
            walk(row.get('children', []), name)
    walk(data['blockdevices'])
    arrays = {}
    for name, row in devices.items():
        if row['type'] != 'raid0':
            continue
        try:
            md = Path('/sys/class/block') / Path(row['kname']).name / 'md'
            metadata = (md / 'metadata_version').read_text().strip()
            container = metadata.split('/')[1] if metadata.startswith('external:/') else ''
            imsm = bool(container) and (md.parent.parent / container / 'md/metadata_version').read_text().strip() == 'external:imsm'
            states = [(p / 'state').read_text().strip().split(',') for p in md.glob('dev-*')]
            count = int((md / 'raid_disks').read_text())
            healthy = (md / 'level').read_text().strip() == 'raid0' and count >= 2 and len(states) == count and all('in_sync' in s and not {'faulty', 'blocked', 'spare'} & set(s) for s in states)
            arrays[name] = {'imsm': imsm, 'healthy': healthy}
        except (OSError, ValueError, IndexError):
            arrays[name] = {'imsm': False, 'healthy': False}
    return {'uefi': Path('/sys/firmware/efi').is_dir(),
            'live': run('findmnt', '-n', '-o', 'FSTYPE', '/') == 'overlay' and Path('/cdrom/casper').is_dir(),
            'devices': devices, 'arrays': arrays}


def select(snapshot, root_uuid, esp_uuid):
    """Fail closed on ambiguity; no volatile device names accepted as identities."""
    if not snapshot['uefi'] or not snapshot['live']:
        raise ValueError('Boot the prepared USB in UEFI mode and choose Try Zorin OS first.')
    def one(uuid):
        if not FS_UUID.fullmatch(uuid):
            raise ValueError('Select a valid filesystem UUID.')
        found = [d for d in snapshot['devices'].values() if (d.get('uuid') or '').lower() == uuid.lower()]
        if len(found) != 1:
            raise ValueError('Filesystem UUID missing or duplicated. Refresh and inspect the layout.')
        d = found[0]
        if d.get('ro') or d.get('type') != 'part' or any(d.get('mountpoints') or []):
            raise ValueError('Selected partition is read-only, mounted, or not a direct partition. Close the installer and unmount it first.')
        return d
    root, esp = one(root_uuid), one(esp_uuid)
    parent = root['parent']
    array = snapshot['arrays'].get(parent, {})
    if not array.get('imsm') or not array.get('healthy'):
        raise ValueError('Expected a complete IMSM RAID 0 volume with every member in sync. Stop; do not force assembly.')
    if snapshot['devices'].get(parent, {}).get('pttype') != 'gpt':
        raise ValueError('Only an existing GPT RAID volume is supported.')
    if root.get('fstype') != 'ext4' or (root.get('parttype') or '').lower() != LINUX or int(root['size']) < 32 * 1024**3:
        raise ValueError('Root must be an existing Linux ext4 partition of at least 32 GiB on the RAID volume.')
    if esp['parent'] != parent or esp.get('fstype') != 'vfat' or (esp.get('parttype') or '').lower() != ESP:
        raise ValueError('Select the existing FAT EFI System Partition on the same RAID volume. Never format it.')
    return root, esp


def plan(snapshot, root_uuid, esp_uuid, mode):
    if mode not in ('dual', 'single'):
        raise ValueError('Unknown installation mode')
    root, esp = select(snapshot, root_uuid, esp_uuid)
    return f'''PERSONALIZED INSTALLATION PLAN — review every installer change

Mode: {'Preserve Windows (dual boot)' if mode == 'dual' else 'Linux only (no permission to erase other partitions)'}
Assembled RAID volume: {root['parent']}
Linux root: {root['name']} — {int(root['size']) / 1024**3:.1f} GiB — UUID {root_uuid}
Existing EFI partition: {esp['name']} — UUID {esp_uuid}

1. Back up files from the selected Linux partition. Replacing it and selecting Format erases ALL files on that partition. Confirm its size and identity, not just its current device name.
2. Open Install Zorin OS and choose Something else (manual partitioning).
3. For the selected Linux partition only, choose ext4 and mount point /. Select Format only when intentionally replacing its contents. Do not select any other partition for formatting.
4. Use the existing EFI System Partition above for EFI boot (/boot/efi). Leave formatting OFF. Preserve Windows, MSR, recovery and all other partitions.
5. If a bootloader device selector is offered, use the assembled RAID volume above, never an individual NVMe/RAID member or the USB. The EFI partition choice must still match this plan. If the installer cannot use it or asks to format it, STOP.
6. Review the installer's pending changes before Install Now. This assistant cannot enforce choices in the external installer. Never choose Erase disk, create a new partition table, or change RAID/AHCI mode.
7. At completion choose Continue Testing, not Restart. Close the installer. Refresh this assistant: formatting changes the root filesystem UUID. Select the newly installed root and the same EFI partition, then use Finalize installed system.

The assistant never partitions or formats. It cannot infer which ext4 data is disposable. This version supports existing direct ext4 root partitions and an existing ESP on one complete GPT IMSM volume. For new space, resizing, encryption, LVM, separate /boot, or multiple ESPs needing a different layout, stop for a reviewed manual plan.

Finalization checks Windows boot files in dual mode, writes UUID-based mdadm configuration, rebuilds initramfs and checks GRUB. It does not install a missing EFI loader or change firmware boot order. Physical UEFI and Windows/Linux boot testing is still required.
'''


def finalize(root_uuid, esp_uuid, mode):
    """Called only as root in a private mount namespace, from the live session."""
    if os.geteuid() != 0:
        raise ValueError('Administrator authentication is required.')
    if os.readlink('/proc/self/ns/mnt') == os.readlink('/proc/1/ns/mnt'):
        raise ValueError('Finalization requires a private mount namespace.')
    root, esp = select(inventory(), root_uuid, esp_uuid)
    print(plan(inventory(), root_uuid, esp_uuid, mode))
    phrase = 'PREPARE ' + root_uuid
    if input(f'To mount the installed Linux system for finalization, type {phrase}: ') != phrase:
        raise ValueError('Confirmation did not match; nothing mounted.')
    # Refresh after confirmation to reject removal, duplicate UUIDs or changed mounts.
    root, esp = select(inventory(), root_uuid, esp_uuid)
    mounts = []
    target = Path(tempfile.mkdtemp(prefix='axentra-installed-', dir='/mnt'))
    try:
        def mount(*args):
            subprocess.run(['mount', *map(str, args)], check=True)
            mounts.append(str(args[-1]))
        try:
            # Inspect without replaying the ext4 journal or modifying the ESP.
            mount('-t', 'ext4', '-o', 'ro,noload', '/dev/disk/by-uuid/' + root_uuid, target)
            release = (target / 'etc/os-release').read_text()
            if not re.search(r'^ID="?zorin"?$', release, re.M):
                raise ValueError('Selected root does not contain installed Zorin.')
            entries = run('findmnt', '--fstab', '--tab-file', str(target / 'etc/fstab'), '-rn', '-o', 'SOURCE,TARGET').splitlines()
            boot = {}
            for line in entries:
                spec, dest = line.split()
                if dest in ('/', '/boot', '/boot/efi'):
                    if dest in boot:
                        raise ValueError('Duplicate boot mount in fstab.')
                    boot[dest] = spec.lower()
            if boot != {'/': 'uuid=' + root_uuid.lower(), '/boot/efi': 'uuid=' + esp_uuid.lower()}:
                raise ValueError('fstab does not match this root/ESP, or uses unsupported separate /boot. No changes made.')
            # Refuse symlink mount destinations from the installed filesystem.
            for rel in ('dev', 'proc', 'sys', 'run', 'boot', 'boot/efi'):
                p = target / rel
                if not p.is_dir() or p.is_symlink() or not p.resolve().is_relative_to(target):
                    raise ValueError('Unsafe or missing installed mount directory: ' + rel)
            subprocess.run(['umount', str(target)], check=True)
            mounts.pop()
            mount('-t', 'ext4', '/dev/disk/by-uuid/' + root_uuid, target)
            mount('-t', 'vfat', '-o', 'ro', '/dev/disk/by-uuid/' + esp_uuid, target / 'boot/efi')
            for name in ('dev', 'proc', 'sys', 'run'):
                mount('--rbind', '/' + name, target / name)
                subprocess.run(['mount', '--make-rslave', str(target / name)], check=True)
            subprocess.run([FINALIZER, '--target', str(target), '--mode', mode], check=True)
            print('Finalizer succeeded. Now test firmware boot, Linux and Windows manually. No reboot performed.')
        finally:
            for path in reversed(mounts):
                # No lazy/forced unmount: failures must remain visible.
                subprocess.run(['umount', '--recursive', path], check=True)
    finally:
        # Never recursively delete a directory that may still contain mounts.
        try:
            target.rmdir()
        except OSError:
            print('Mount directory retained for inspection: ' + str(target))


def gui():
    import tkinter as tk
    from tkinter import ttk, messagebox
    from tkinter.scrolledtext import ScrolledText
    window = tk.Tk()
    window.title('Axentra Installation Assistant — experimental')
    window.geometry('1040x780')
    frame = ttk.Frame(window, padding=16)
    frame.pack(fill='both', expand=True)
    ttk.Label(frame, text='Install Zorin on Intel RAID', font=('', 20, 'bold')).pack(anchor='w')
    ttk.Label(frame, text='Offline guidance • No automatic partitioning • Preserve the existing EFI partition').pack(anchor='w', pady=8)
    status = tk.StringVar()
    ttk.Label(frame, textvariable=status, wraplength=980).pack(anchor='w')
    mode = tk.StringVar(value='dual')
    modes = ttk.Frame(frame)
    modes.pack(fill='x', pady=8)
    ttk.Radiobutton(modes, text='Preserve Windows', variable=mode, value='dual').pack(side='left')
    ttk.Radiobutton(modes, text='Linux only — preserve other partitions', variable=mode, value='single').pack(side='left', padx=20)
    root_box = ttk.Combobox(frame, state='readonly', width=110)
    esp_box = ttk.Combobox(frame, state='readonly', width=110)
    ttk.Label(frame, text='Linux root (existing ext4; replacement may erase its files)').pack(anchor='w')
    root_box.pack(fill='x', pady=4)
    ttk.Label(frame, text='Existing EFI partition (never format)').pack(anchor='w')
    esp_box.pack(fill='x', pady=4)
    output = ScrolledText(frame, wrap='word', height=25)
    output.pack(fill='both', expand=True, pady=10)
    choices = {}
    def display(text):
        output.configure(state='normal')
        output.delete('1.0', 'end')
        output.insert('1.0', text)
        output.configure(state='disabled')
    def refresh():
        try:
            snap = inventory()
            choices.clear()
            root_box.set('')
            esp_box.set('')
            roots, esps, rows = [], [], []
            for d in snap['devices'].values():
                rows.append(f"{d['name']} | {int(d['size']) / 1024**3:.1f} GiB | {d.get('fstype') or d['type']} | UUID {d.get('uuid') or '—'} | mounted: {d.get('mountpoints')}")
                if d['type'] == 'part' and d.get('uuid'):
                    label = f"{d['name']} — {int(d['size']) / 1024**3:.1f} GiB — {d['uuid']}"
                    choices[label] = d['uuid']
                    if d.get('fstype') == 'ext4':
                        roots.append(label)
                    if (d.get('parttype') or '').lower() == ESP:
                        esps.append(label)
            root_box['values'], esp_box['values'] = roots, esps
            status.set(f"UEFI: {snap['uefi']} | Live USB session: {snap['live']} | Select partitions explicitly; refresh after installation.")
            display('READ-ONLY DISK INVENTORY\n\n' + '\n'.join(rows) + '\n\nChoose the root and existing ESP, then Review plan. If there is no suitable existing root partition, stop for a reviewed manual partitioning plan. Windows/data partitions and RAID members must be preserved.\n\nAfter installing: choose Continue Testing, close the installer, unmount target partitions, refresh and select their current UUIDs. Finalize installed system prepares temporary mounts, then runs the guarded finalizer in a terminal. It requires administrator authentication and typed confirmations. EFI is mounted read-only. No reboot is automatic.')
        except Exception as exc:
            choices.clear()
            display('Inspection failed; stop installation.\n' + str(exc))
    def review(final=False):
        try:
            root_uuid, esp_uuid = choices[root_box.get()], choices[esp_box.get()]
            text = plan(inventory(), root_uuid, esp_uuid, mode.get())
            display(text)
            if final and messagebox.askyesno('Finalize installed Zorin?', 'Continue only after the installer has finished and closed. This updates the selected installed Linux system, including mdadm, initramfs and GRUB. The terminal will ask for UUID confirmation. It does not partition, format or reboot.'):
                subprocess.Popen(['x-terminal-emulator', '-e', 'sudo', 'unshare', '--mount', '--propagation', 'private', '--', SELF, '--finalize', root_uuid, esp_uuid, mode.get()])
        except Exception as exc:
            messagebox.showerror('Stop and review', str(exc) if not isinstance(exc, KeyError) else 'Select both partitions explicitly.')
    buttons = ttk.Frame(frame)
    buttons.pack(fill='x')
    ttk.Button(buttons, text='Refresh disks', command=refresh).pack(side='left')
    ttk.Button(buttons, text='Review plan', command=review).pack(side='left', padx=10)
    ttk.Button(buttons, text='Finalize installed system', command=lambda: review(True)).pack(side='right')
    refresh()
    window.mainloop()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inventory', action='store_true')
    parser.add_argument('--finalize', nargs=3, metavar=('ROOT_UUID', 'ESP_UUID', 'MODE'))
    args = parser.parse_args()
    if args.finalize:
        try:
            finalize(*args.finalize)
        except Exception as exc:
            print('STOP — finalization failed. Do not reboot.\n' + str(exc))
            raise SystemExit(1)
        finally:
            input('Press Enter to close this terminal. ')
    elif args.inventory:
        print(json.dumps(inventory(), indent=2))
    else:
        gui()
