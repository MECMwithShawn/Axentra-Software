#!/usr/bin/env python3
"""Register checksum-verified local Linux packages in Command.

This writes only app definitions and public package metadata/icons. It never
installs an application, publishes a release, or copies private application code.
"""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import xml.etree.ElementTree as ET


def contents(package):
    manifests, desktops, icons = {}, {}, {}
    process = subprocess.Popen(['dpkg-deb', '--fsys-tarfile', str(package)], stdout=subprocess.PIPE)
    with tarfile.open(fileobj=process.stdout, mode='r|') as archive:
        for item in archive:
            name = '/' + item.name.removeprefix('./')
            if not item.isfile() or item.size > 4 * 1024 * 1024:
                continue
            if name.startswith('/usr/share/axentra/command/apps/') and name.endswith('.json'):
                manifests[name] = json.load(archive.extractfile(item))
            elif name.startswith('/usr/share/applications/') and name.endswith('.desktop'):
                desktops[name] = archive.extractfile(item).read().decode()
            elif name.endswith(('.png', '.svg')):
                icons[name] = archive.extractfile(item).read()
    if process.wait() != 0:
        raise RuntimeError(f'Could not read {package}')
    return manifests, desktops, icons


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--packages-root', type=Path, required=True)
    parser.add_argument('--sources-root', type=Path, required=True)
    parser.add_argument('--command-home', type=Path, required=True)
    parser.add_argument('--catalog-root', type=Path, required=True,
                        help='Staged usr/share tree to include in the Command package')
    args = parser.parse_args()
    public = Path(__file__).resolve().parents[1]
    sources = args.sources_root.resolve()
    definitions = sources/'axentra-command/apps'
    specs = [
        ('command', 'command', 'axentra-command'),
        ('crystalwire', 'crystalwire', 'axentra-crystalwire'),
        ('ghost', 'ghost', 'axentra-ghost'),
        ('mail', 'mail', 'axentra-mail'),
        ('vault', 'vault', 'axentra-vault'),
        ('archive', 'zip', 'axentra-zip'),
        ('usb', 'usb', None),
        ('pdf', 'pdf', 'axentra-pdf'),
        ('spaces', 'spaces', 'axentra-spaces'),
        ('horizon', 'horizon', 'stillpoint'),
    ]
    display_names = {'command': 'Command', 'crystalwire': 'CrystalWire',
                     'ghost': 'Ghost', 'mail': 'Mail', 'vault': 'Vault',
                     'archive': 'Archive', 'usb': 'USB', 'pdf': 'PDF',
                     'spaces': 'Spaces', 'horizon': 'Horizon'}
    catalog = ET.Element('components', version='0.16', origin='axentra-local')
    records = []
    pending = []
    for folder, ident, source in specs:
        output = (args.packages_root/f'{folder}-linux').resolve()
        checksum_lines = (output/'SHA256SUMS.txt').read_text().splitlines()
        packages = []
        for line in checksum_lines:
            digest, filename = line.split(maxsplit=1)
            filename = filename.lstrip('*')
            if Path(filename).name != filename:
                raise ValueError(f'Invalid package filename: {filename}')
            if not filename.endswith('.deb'):
                continue
            package = output/filename
            actual = hashlib.sha256(package.read_bytes()).hexdigest()
            if digest != actual:
                raise ValueError(f'Checksum mismatch: {package}')
            packages.append((package, actual))
        if len(packages) != 1:
            raise ValueError(f'Expected one current package in {output}/SHA256SUMS.txt')
        package, digest = packages[0]
        name, version, arch = subprocess.check_output(
            ['dpkg-deb', '--show', '--showformat=${Package}\\n${Version}\\n${Architecture}', str(package)],
            text=True).strip().splitlines()
        if arch != 'amd64' or not name.startswith('axentra-'):
            raise ValueError(f'Unexpected package identity: {package}')
        manifests, desktops, icons = contents(package)
        app = json.loads((definitions/f'{ident}.json').read_text())
        destination = args.command_home/'apps'/f'{ident}.json'
        if destination.is_file():
            app.update(json.loads(destination.read_text()))
        packaged = next((v for v in manifests.values() if v['id'] == ident), {})
        app.update(packaged)
        if ident == 'command':
            app.update(install={'exe': '/opt/axentra/command/AxentraCommand', 'uninstall_key': name},
                       launch={'exe': '/opt/axentra/command/AxentraCommand'}, process=['AxentraCommand'])
        if ident == 'usb':
            app.update(install={'exe': '/usr/bin/axentra-usb', 'uninstall_key': name},
                       launch={'exe': '/usr/bin/axentra-usb'}, process=['axentra-usb'],
                       icon='/usr/share/icons/hicolor/scalable/apps/axentra-usb.svg')
        if ident == 'pdf':
            app.update(name='Axentra PDF (Rust preview)', tagline='PDF editor — Rust migration in progress',
                       install={'exe': '/opt/axentra/pdf-preview/AxentraPDF', 'uninstall_key': name},
                       launch={'exe': '/opt/axentra/pdf-preview/AxentraPDF'}, process=['AxentraPDF'],
                       icon='/usr/share/icons/hicolor/256x256/apps/axentra-pdf-preview.png')
        source_dir = sources/source if source else public/'USB/source'
        app['source'] = {'path': str(source_dir), 'version_file':
                        'manifest.json' if ident == 'horizon' else
                        'app/Cargo.toml' if ident == 'vault' else 'Cargo.toml'}
        app['dist'] = {'path': str(output), 'prefix': name+'_', 'suffix': '_amd64.deb'}
        app['docs'] = str(public/('Edge-Extension-Suite/Horizon' if ident == 'horizon' else
                                 display_names[folder]))
        if app.get('release'):
            app['release']['asset_suffix'] = '_amd64.deb'
        icon_path = app.get('icon', '')
        if icon_path not in icons:
            icon_path = f'/usr/share/icons/hicolor/256x256/apps/{name}.png'
        if icon_path not in icons:
            raise ValueError(f'No packaged icon for {ident}')
        catalog_icon = Path(f'/usr/share/axentra/command/catalog-icons/{ident}{Path(icon_path).suffix}')
        staged_icon = args.catalog_root/catalog_icon.relative_to('/')
        staged_icon.parent.mkdir(parents=True, exist_ok=True)
        staged_icon.write_bytes(icons[icon_path])
        # The catalog icon exists before the target app is installed.
        app['icon'] = str(catalog_icon)
        desktop = next(iter(desktops), None)
        component = ET.SubElement(catalog, 'component', type='desktop-application' if desktop else 'generic')
        for key, value in [
            # PackageKit identifies a loose .deb by its desktop file. GNOME
            # Software 46 does not fall back to pkgname once that ID is set.
            ('id', Path(desktop).name if desktop else f'io.github.MECMwithShawn.Axentra{display_names[folder]}'),
            ('name', app['name']), ('summary', app['tagline'].rstrip('.')), ('pkgname', name),
            ('metadata_license', 'CC0-1.0'),
        ]:
            ET.SubElement(component, key).text = value
        developer = ET.SubElement(component, 'developer', id='io.github.mecmwithshawn')
        ET.SubElement(developer, 'name').text = 'Axentra Software'
        ET.SubElement(component, 'icon', type='local').text = str(catalog_icon)
        if desktop:
            ET.SubElement(component, 'launchable', type='desktop-id').text = Path(desktop).name
        ET.SubElement(component, 'url', type='homepage').text = 'https://github.com/MECMwithShawn/Axentra-Software'
        description = ET.SubElement(component, 'description')
        ET.SubElement(description, 'p').text = app['tagline'].rstrip('.') + '. Local Linux package for the Axentra suite.'
        pending.append((destination, app))
        records.append({'id': ident, 'package': str(package), 'version': version, 'sha256': digest,
                        'icon': str(catalog_icon), 'preview': ident == 'pdf'})
    metadata = args.catalog_root/'usr/share/swcatalog/xml/axentra-local.xml'
    metadata.parent.mkdir(parents=True, exist_ok=True)
    ET.indent(catalog, space='  ')
    ET.ElementTree(catalog).write(metadata, encoding='utf-8', xml_declaration=True)
    subprocess.run(['appstreamcli', 'validate', '--no-net',
                    '--override=cid-desktopapp-is-not-rdns=info', str(metadata)], check=True)
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
    for destination, app in pending:
        destination.parent.mkdir(parents=True, exist_ok=True)
        if destination.exists():
            shutil.copy2(destination, destination.with_suffix('.json.backup-'+stamp))
        temporary = destination.with_suffix('.tmp')
        temporary.write_text(json.dumps(app, indent=2)+'\n')
        temporary.chmod(0o600)
        temporary.replace(destination)
    (args.catalog_root/'packages.json').write_text(json.dumps(records, indent=2)+'\n')
    print(f'Registered {len(records)} verified packages; PDF is explicitly a preview.')


if __name__ == '__main__':
    main()
