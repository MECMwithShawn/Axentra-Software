"""Development packaging only. No Python is included in native app releases."""
import argparse
import hashlib
import io
from pathlib import Path
import struct
import tarfile
import zipfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
VERSION = tomllib.loads((ROOT/'Cargo.toml').read_text(encoding='utf-8'))['package']['version']
DEPENDS = 'libc6 (>= 2.35), libgcc-s1, libgl1, libegl1, libx11-6, libxcursor1, libxi6, libxrandr2, libxkbcommon0, libwayland-client0, util-linux, parted, dosfstools, ntfs-3g, udev, pkexec'


def tar(entries):
    out = io.BytesIO()
    with tarfile.open(fileobj=out, mode='w:gz') as archive:
        for name, data, mode in entries:
            item = tarfile.TarInfo(name)
            item.size = len(data); item.mode = mode; item.mtime = 0
            item.uid = item.gid = 0; item.uname = item.gname = 'root'
            archive.addfile(item, io.BytesIO(data))
    return out.getvalue()


def ar(name, data):
    header = ('%-16s%-12s%-6s%-6s%-8s%-10s`\n' % (name+'/', 0, 0, 0, '100644', len(data))).encode()
    return header + data + (b'\n' if len(data) % 2 else b'')


def build(windows, linux, out):
    out.mkdir(parents=True, exist_ok=True)
    win = windows.read_bytes() if windows else None
    elf = linux.read_bytes() if linux else None
    if win: assert win[:2] == b'MZ', 'Windows binary is not a PE executable'
    if elf: assert elf[:6] == b'\x7fELF\x02\x01' and struct.unpack_from('<H', elf, 18)[0] == 62, 'Linux binary is not x86-64 ELF'
    docs = [(p.name, p.read_bytes(), 0o644) for p in [ROOT/'README.md', ROOT/'LICENSE', ROOT/'VALIDATION.txt', ROOT/'THIRD-PARTY-NOTICES.txt']]
    if win:
        winzip = out/f'AxentraUSB-{VERSION}-Windows-x64.zip'
        with zipfile.ZipFile(winzip, 'w', zipfile.ZIP_DEFLATED, compresslevel=9) as z:
            z.writestr('AxentraUSB/AxentraUSB.exe', win)
            for name, data, _ in docs: z.writestr('AxentraUSB/'+name, data)
        with zipfile.ZipFile(winzip, 'a', zipfile.ZIP_DEFLATED) as z:
            z.writestr('AxentraUSB/icons/usb-outline-v1.ico', (ROOT/'assets/axentra-usb.ico').read_bytes())
    if elf:
        (out/f'AxentraUSB-{VERSION}-Linux-x64.tar.gz').write_bytes(tar([('AxentraUSB/axentra-usb',elf,0o755)]+[('AxentraUSB/'+n,b,m) for n,b,m in docs]))
        control = f'''Package: axentra-usb
Version: {VERSION}
Section: utils
Priority: optional
Architecture: amd64
Maintainer: Axentra
Depends: {DEPENDS}
Recommends: xdg-desktop-portal, xdg-desktop-portal-gtk
Description: Native Rust USB image writer with boot diagnostics (preview)
 Inspects, creates and verifies original-image and reviewed Zorin USB media.
 Hardware boot and native device writes require validation on a spare USB.
'''.encode()
        payload = [('usr/bin/axentra-usb',elf,0o755),('usr/share/applications/axentra-usb.desktop',(ROOT/'packaging/axentra-usb.desktop').read_bytes(),0o644),('usr/share/icons/hicolor/scalable/apps/axentra-usb.svg',(ROOT/'packaging/axentra-usb.svg').read_bytes(),0o644)]
        payload += [('usr/share/doc/axentra-usb/'+n,b,m) for n,b,m in docs]
        deb = b'!<arch>\n'+ar('debian-binary',b'2.0\n')+ar('control.tar.gz',tar([('control',control,0o644)]))+ar('data.tar.gz',tar(payload))
        (out/f'axentra-usb_{VERSION}_amd64.deb').write_bytes(deb)
    source = [(f'AxentraUSB-Rust/{p.relative_to(ROOT).as_posix()}',p.read_bytes(),0o644) for p in ROOT.rglob('*') if p.is_file() and not any(x in p.parts for x in ('target','dist','__pycache__','.git'))]
    with zipfile.ZipFile(out/f'AxentraUSB-{VERSION}-Rust-source.zip','w',zipfile.ZIP_DEFLATED,compresslevel=9) as z:
        for name,b,_ in source:z.writestr(name,b)
    files = [p for p in out.iterdir() if p.suffix in ('.zip','.gz','.deb','.exe') and VERSION in p.name]
    (out/'SHA256SUMS.txt').write_text(''.join(hashlib.sha256(p.read_bytes()).hexdigest()+'  '+p.name+'\n' for p in sorted(files)),encoding='ascii')
    print('\n'.join(f'{p.name}: {p.stat().st_size:,} bytes' for p in files))


if __name__ == '__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('--windows',type=Path)
    parser.add_argument('--linux',type=Path)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    if not (args.windows or args.linux): parser.error('Provide --windows and/or --linux')
    build(args.windows,args.linux,args.output)
