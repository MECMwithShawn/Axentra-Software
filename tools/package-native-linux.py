#!/usr/bin/env python3
"""Build Zorin 18 / Ubuntu 24.04 packages from separately held Rust sources.

Build tooling only. The packages contain native Rust applications and their
existing interfaces; private application sources are not copied into this repo.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib
import xml.etree.ElementTree as ET

APPS = {
    'ghost': dict(name='Axentra Ghost', binary='AxentraGhost', icon='assets/ghost.ico',
                  tagline='Linux privacy controls with recorded undo and drift watch', category='Utility;Security;', tray=True),
    'crystalwire': dict(name='Axentra CrystalWire', binary='CrystalWire', icon='assets/crystalwire-128.png',
                       tagline='Network monitor, DNS blocking and Pi-hole', category='Network;Monitor;', tray=True),
    'spaces': dict(name='Axentra Spaces', binary='AxentraSpaces', icon='spaces-512.png',
                   tagline='Borderless, always-on-top browser windows', category='Network;',
                   mime='x-scheme-handler/axentra-spaces;', argument='%u', tray=True),
    'mail': dict(name='Axentra Mail', binary='AxentraMail', icon='assets/mail-256.png',
                 tagline='Outlook.com with private, on-device AI triage', category='Network;Email;', tray=True),
    'vault': dict(name='Axentra Vault', binary='AxentraVault', built='axentra-vault',
                  icon='app/assets/app/vault-256.png', cargo='app/Cargo.toml',
                  tagline='KeePass-compatible password manager', category='Utility;Security;'),
    'archive': dict(name='Axentra Archive', id='zip', binary='AxentraZip', icon='assets/zip-256.png',
                    tagline='Your files, neatly packed. Powered by 7-Zip.', category='Utility;Archiving;',
                    mime='application/zip;application/x-7z-compressed;application/vnd.rar;', argument='%f'),
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--app', required=True, choices=APPS)
    parser.add_argument('--source', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--revision', type=int, default=1,
                        help='Debian packaging revision (application version is unchanged)')
    args = parser.parse_args()
    if args.revision < 1:
        parser.error('--revision must be positive')
    app = APPS[args.app]
    source = args.source.resolve(strict=True)
    metadata = tomllib.loads((source/app.get('cargo', 'Cargo.toml')).read_text())
    version = metadata.get('package', metadata.get('workspace', {}).get('package', {}))['version']
    binary = source/'target/release'/app.get('built', app['binary'])
    if not binary.is_file():
        raise SystemExit('Build the native application with cargo build --release --locked first')
    elf = binary.read_bytes()[:20]
    if elf[:6] != b'\x7fELF\x02\x01' or elf[18:20] != b'\x3e\x00':
        raise SystemExit('Expected a native Linux x86_64 ELF executable')
    args.output.mkdir(parents=True, exist_ok=True)
    output = args.output.resolve()
    package = f'axentra-{args.app}'
    dependencies = ['libc6 (>= 2.39)', 'libgtk-3-0t64', 'libayatana-appindicator3-1', 'libxdo3', 'xdg-utils']
    if args.app == 'vault':
        dependencies += ['libfido2-1', 'libegl1', 'libgl1', 'libxkbcommon0', 'zenity']
    else:
        dependencies += ['libwebkit2gtk-4.1-0']
    if args.app == 'crystalwire':
        dependencies += ['libssl3t64', 'gnome-keyring', 'curl', 'iproute2', 'network-manager', 'nftables', 'pkexec', 'libnotify-bin', 'systemd', 'init-system-helpers', 'clamav']
    if args.app == 'ghost':
        dependencies += ['pkexec', 'libnotify-bin', 'systemd', 'init-system-helpers']
    if args.app == 'mail':
        dependencies += ['libssl3t64', 'gnome-keyring']
    with tempfile.TemporaryDirectory(prefix=f'{package}-build-') as td:
        stage = Path(td)
        installed = Path('/opt/axentra')/args.app
        app_dir = stage/installed.relative_to('/')
        app_dir.mkdir(parents=True)
        target = app_dir/app['binary']
        shutil.copy2(binary, target)
        (app_dir/'VERSION').write_text(version+'\n')
        extra_executables = []
        if args.app == 'ghost':
            units = stage/'usr/lib/systemd/system'
            units.mkdir(parents=True)
            (units/'axentra-ghost-watch.service').write_text('''[Unit]
Description=Check Axentra Ghost managed privacy controls
[Service]
Type=oneshot
ExecStart=/opt/axentra/ghost/AxentraGhost --helper watch-check
User=root
UMask=0022
TimeoutStartSec=5min
NoNewPrivileges=yes
PrivateTmp=yes
ProtectHome=yes
ProtectSystem=full
ReadWritePaths=/etc /var/lib/axentra/ghost
''')
            (units/'axentra-ghost-watch.timer').write_text('''[Unit]
Description=Check Axentra Ghost privacy controls after boot and daily
[Timer]
OnBootSec=2min
OnCalendar=*-*-* 12:30:00
Persistent=yes
Unit=axentra-ghost-watch.service
[Install]
WantedBy=timers.target
''')
        if args.app == 'crystalwire':
            for name in ['cw-svc', 'cw', 'cw-probe']:
                src = source/'target/release'/name
                if not src.is_file() or src.read_bytes()[:6] != b'\x7fELF\x02\x01':
                    raise SystemExit(f'Native Rust executable is missing: {name}')
                dest = app_dir/name
                shutil.copy2(src, dest)
                extra_executables.append(dest)
            service = stage/'usr/lib/systemd/system/axentra-crystalwire.service'
            service.parent.mkdir(parents=True)
            service.write_text('''[Unit]
Description=Axentra CrystalWire network recording service
After=network.target NetworkManager.service
[Service]
Type=simple
ExecStart=/opt/axentra/crystalwire/cw-svc service
User=root
Group=adm
UMask=0027
RuntimeDirectory=axentra/crystalwire
RuntimeDirectoryMode=0750
StateDirectory=axentra/crystalwire
StateDirectoryMode=0750
Restart=on-failure
RestartSec=5
TimeoutStopSec=25
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=read-only
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectControlGroups=yes
PrivateTmp=yes
RestrictSUIDSGID=yes
LockPersonality=yes
CapabilityBoundingSet=CAP_NET_RAW CAP_NET_ADMIN CAP_DAC_READ_SEARCH CAP_SYS_PTRACE
ReadWritePaths=/var/lib/axentra/crystalwire /run/axentra/crystalwire
[Install]
WantedBy=multi-user.target
''')

        doc = stage/'usr/share/doc'/package
        doc.mkdir(parents=True)
        for name in ['LICENSE', 'COPYING', 'README.md']:
            if (source/name).is_file():
                shutil.copy2(source/name, doc/name)
        if args.app == 'archive':
            (app_dir/'engine').mkdir()
            shutil.copy2(source/'vendor/7zip-linux/7zz', app_dir/'engine/7zz')
            shutil.copy2(source/'vendor/7zip-linux/License.txt', doc/'7-Zip-License.txt')
            shutil.copy2(source/'vendor/7zip-linux/readme.txt', doc/'7-Zip-readme.txt')
        executable = f'/usr/bin/{package}'
        launcher = stage/executable.lstrip('/')
        launcher.parent.mkdir(parents=True)
        launcher.symlink_to(installed/app['binary'])
        desktop = stage/'usr/share/applications'/f'{package}.desktop'
        desktop.parent.mkdir(parents=True)
        desktop.write_text('\n'.join([
            '[Desktop Entry]', 'Type=Application', f'Name={app["name"]}',
            f'Comment={app["tagline"]}', f'Exec={executable} {app.get("argument", "")}'.rstrip(),
            f'Icon={package}', 'Terminal=false', f'Categories={app["category"]}',
            f'StartupWMClass={package if args.app == "vault" else "Axentra-"+args.app}',
            *([f'MimeType={app["mime"]}'] if app.get('mime') else []), '',
        ]))
        subprocess.run(['desktop-file-validate', str(desktop)], check=True)
        icon = stage/'usr/share/icons/hicolor/256x256/apps'/f'{package}.png'
        icon.parent.mkdir(parents=True)
        subprocess.run(['/usr/bin/python3', '-c',
                        "import gi,sys;gi.require_version('GdkPixbuf','2.0');from gi.repository import GdkPixbuf;p=GdkPixbuf.Pixbuf.new_from_file_at_scale(sys.argv[1],256,256,True);p.savev(sys.argv[2],'png',[],[])",
                        str(source/app['icon']), str(icon)], check=True)
        if args.app == 'vault':
            # Software stores need AppStream metadata in addition to the
            # desktop launcher and icon. A loose .deb alone has only package
            # fields before installation/catalog discovery.
            component_id = 'io.github.MECMwithShawn.AxentraVault'
            component = ET.Element('component', type='desktop-application')
            for key, value in [
                ('id', component_id), ('metadata_license', 'CC0-1.0'),
                ('project_license', 'GPL-3.0-or-later'),
                ('name', app['name']), ('summary', app['tagline']),
                ('pkgname', package),
            ]:
                ET.SubElement(component, key).text = value
            developer = ET.SubElement(component, 'developer', id='io.github.mecmwithshawn')
            ET.SubElement(developer, 'name').text = 'Axentra Software'
            ET.SubElement(component, 'launchable', type='desktop-id').text = desktop.name
            ET.SubElement(component, 'icon', type='stock').text = package
            ET.SubElement(component, 'url', type='homepage').text = 'https://github.com/MECMwithShawn/Axentra-Software'
            description = ET.SubElement(component, 'description')
            ET.SubElement(description, 'p').text = (
                'Axentra Vault is a KeePass-compatible password manager. '
                'Create and unlock password databases, organize entries, and '
                'copy credentials with automatic clipboard clearing.')
            categories = ET.SubElement(component, 'categories')
            for category in app['category'].strip(';').split(';'):
                ET.SubElement(categories, 'category').text = category
            provides = ET.SubElement(component, 'provides')
            ET.SubElement(provides, 'binary').text = package
            metainfo = stage/'usr/share/metainfo'/f'{component_id}.metainfo.xml'
            metainfo.parent.mkdir(parents=True)
            ET.indent(component, space='  ')
            ET.ElementTree(component).write(metainfo, encoding='utf-8', xml_declaration=True)
            subprocess.run(['appstreamcli', 'validate', '--no-net', str(metainfo)], check=True)
        manifest = dict(id=app.get('id', args.app), name=app['name'], kind='desktop',
                        tagline=app['tagline'], icon='/'+str(icon.relative_to(stage)),
                        install=dict(exe=str(installed/app['binary']), uninstall_key=package),
                        launch=dict(exe=str(installed/app['binary'])), process=[app['binary']],
                        docs=f'/usr/share/doc/{package}')
        if app.get('tray'):
            manifest['background_args'] = ['--background']
        registry = stage/'usr/share/axentra/command/apps'/f'{manifest["id"]}.json'
        registry.parent.mkdir(parents=True)
        registry.write_text(json.dumps(manifest, indent=2)+'\n')
        control = stage/'DEBIAN/control'
        control.parent.mkdir()
        if args.app == 'ghost':
            (control.parent/'postinst').write_text('''#!/bin/sh
set -e
if [ "$1" = configure ]; then
  if [ ! -d /var/lib/axentra ]; then install -d -o root -g root -m 0755 /var/lib/axentra; fi
  install -d -o root -g root -m 0755 /var/lib/axentra/ghost
  install -d -o root -g root -m 0700 /var/lib/axentra/ghost/private
  if [ -d /run/systemd/system ]; then systemctl daemon-reload; fi
fi
# Drift watch is enabled only through the user's choice in Ghost.
''')
            (control.parent/'prerm').write_text('''#!/bin/sh
set -e
if [ -d /run/systemd/system ]; then
  deb-systemd-invoke stop axentra-ghost-watch.timer axentra-ghost-watch.service || true
fi
if [ "$1" = remove ]; then deb-systemd-helper disable axentra-ghost-watch.timer >/dev/null || true; fi
''')
            (control.parent/'postrm').write_text('''#!/bin/sh
set -e
if [ -d /run/systemd/system ]; then systemctl daemon-reload; fi
if [ "$1" = purge ]; then deb-systemd-helper purge axentra-ghost-watch.timer >/dev/null || true; fi
# Retain undo records and managed settings; package removal does not undo user choices.
''')
        if args.app == 'crystalwire':
            (control.parent/'postinst').write_text('''#!/bin/sh
set -e
if [ "$1" = configure ]; then
  if [ ! -d /var/lib/axentra ]; then install -d -o root -g root -m 0755 /var/lib/axentra; fi
  install -d -o root -g adm -m 0750 /var/lib/axentra/crystalwire
  install -d -o root -g root -m 0700 /var/lib/axentra/crystalwire/private
  if [ -d /run/systemd/system ]; then
    systemctl daemon-reload
    deb-systemd-helper enable axentra-crystalwire.service >/dev/null || true
    deb-systemd-invoke start axentra-crystalwire.service || true
  fi
fi
''')
            (control.parent/'prerm').write_text('''#!/bin/sh
set -e
if [ -d /run/systemd/system ]; then
  deb-systemd-invoke stop axentra-crystalwire.service || true
fi
if [ "$1" = remove ]; then deb-systemd-helper disable axentra-crystalwire.service >/dev/null || true; fi
''')
            (control.parent/'postrm').write_text('''#!/bin/sh
set -e
if [ -d /run/systemd/system ]; then systemctl daemon-reload; fi
if [ "$1" = purge ]; then deb-systemd-helper purge axentra-crystalwire.service >/dev/null || true; fi
# Recorded history and credentials are retained; uninstall never deletes user data.
''')

        extra = 'Recommends: clamav, clamav-freshclam, xdg-desktop-portal-gtk\n' if args.app == 'mail' else 'Recommends: xdg-desktop-portal-gtk\n'
        control.write_text(f'''Package: {package}
Version: {version}-{args.revision}
Section: utils
Priority: optional
Architecture: amd64
Maintainer: Axentra
Installed-Size: {sum(p.stat().st_size for p in stage.rglob('*') if p.is_file() and not p.is_symlink())//1024}
Depends: {', '.join(dependencies)}
{extra}Description: {app['name']} for Zorin OS 18 / Ubuntu 24.04
 {app['tagline']}
 Native Rust application, preserving the existing interface.
''')
        for path in stage.rglob('*'):
            if not path.is_symlink():
                path.chmod(0o755 if path.is_dir() or path == target or path in extra_executables or path.name in ['postinst', 'prerm', 'postrm'] or path == app_dir/'engine/7zz' else 0o644)
        result = output/f'{package}_{version}-{args.revision}_amd64.deb'
        subprocess.run(['dpkg-deb', '--root-owner-group', '--build', str(stage), str(result)], check=True)
        (output/'SHA256SUMS.txt').write_text(f'{hashlib.sha256(result.read_bytes()).hexdigest()}  {result.name}\n')
        print(result)


if __name__ == '__main__':
    main()
