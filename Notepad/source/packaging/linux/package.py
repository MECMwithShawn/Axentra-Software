#!/usr/bin/env python3
"""Build and package the native Rust binary for Zorin 18 / Ubuntu 24.04 x86_64."""
import argparse, hashlib, json, os, pathlib, re, shutil, stat, subprocess, tempfile, tomllib
EXE='AxentraNotepad'
def installed_kib(tree):
    # dpkg-gencontrol's rule: each regular file or symlink counts its own size in KiB, rounded
    # up, and every directory (the root too) counts 1 KiB. It goes by lstat, so the absolute
    # usr/bin/axentra-notepad symlink counts as a link and is never followed into the build
    # host's /opt. Call it before DEBIAN/ exists; the tree has no hard links to count once.
    return sum(-(-st.st_size//1024) if stat.S_ISREG(st.st_mode) or stat.S_ISLNK(st.st_mode) else 1
               for st in (x.lstat() for x in [tree,*tree.rglob('*')]))
def write_sums(folder,built):
    # One line per .deb in the folder, oldest version first. Lines already in the file are
    # kept, so a damaged older package still fails `sha256sum -c`; the deb just built and
    # any deb not listed yet are hashed.
    sums_file=folder/'SHA256SUMS.txt'; known={}
    if sums_file.is_file():
        for line in sums_file.read_text().splitlines():
            if m:=re.fullmatch(r'([0-9a-f]{64}) [ *](.+)',line.strip()): known[m[2]]=m[1]
    natural=lambda f:[int(t) if t.isdigit() else t for t in re.split(r'(\d+)',f.name)]
    lines=[f'{known[d.name] if d.name in known and d!=built else hashlib.sha256(d.read_bytes()).hexdigest()}  {d.name}\n'
           for d in sorted(folder.glob('*.deb'),key=natural)]
    tmp=folder/'.SHA256SUMS.txt.tmp'; tmp.write_text(''.join(lines)); tmp.replace(sums_file)
def cargo_build(root):
    # cargo build --release --locked, into CARGO_TARGET_DIR if it's set (a relative one counts from
    # where package.py was started, as it would for cargo) and target/ otherwise. Cargo decides
    # what's stale, so the binary it reports matches the sources: ui/ (build.rs's rerun-if-changed),
    # build.rs, the Rust files, and the Cargo.toml/Cargo.lock edits that change the build, such as
    # [profile.release] settings, dependencies or the version. --locked stops the build if
    # Cargo.lock doesn't match Cargo.toml. Returns the binary's path and whether cargo left it as is.
    env=dict(os.environ)
    if env.get('CARGO_TARGET_DIR'): env['CARGO_TARGET_DIR']=os.path.abspath(env['CARGO_TARGET_DIR'])
    run=subprocess.run(['cargo','build','--release','--locked','--message-format=json-render-diagnostics'],
                       cwd=root,env=env,stdout=subprocess.PIPE,text=True)
    if run.returncode: raise SystemExit('cargo build --release --locked failed')
    for line in run.stdout.splitlines():
        msg=json.loads(line) if line.startswith('{') else {}
        if msg.get('reason')=='compiler-artifact' and msg['target']['name']==EXE and msg.get('executable'):
            return pathlib.Path(msg['executable']),msg['fresh']
    raise SystemExit(f'cargo build --release --locked did not report the {EXE} binary')
p=argparse.ArgumentParser()
p.add_argument('--output',type=pathlib.Path,required=True)
p.add_argument('--revision',type=int,default=1)
a=p.parse_args(); root=pathlib.Path(__file__).resolve().parents[2]
if a.revision < 1: p.error('--revision must be positive')
a.output.mkdir(parents=True,exist_ok=True); a.output=a.output.resolve()
# The page is compiled into the binary, and build.rs only warns about a missing page file (so the
# backend and its tests still build), so a checkout without the whole page is refused here.
sources=[root/'ui'/n for n in ('index.html','app.css','app.js','core.js','editor.js','lang.js')]
missing=[s.name for s in sources if not s.is_file() or s.stat().st_size==0]
if missing: raise SystemExit('ui/ is missing '+', '.join(missing))
with tempfile.TemporaryDirectory(prefix='notepad-package-') as td:
    stage=pathlib.Path(td); app=stage/'opt/axentra/notepad'; app.mkdir(parents=True)
    # A file saved while cargo was building can miss that build, so after copying the binary, ask
    # cargo again: if it leaves the binary as is, the copy matches the sources as they are now; if
    # it rebuilds, copy the new binary and ask again.
    binary,_=cargo_build(root)
    for _ in range(3):
        shutil.copy2(binary,app/EXE)
        binary,fresh=cargo_build(root)
        if fresh: break
    else: raise SystemExit('The sources kept changing during the build; run package.py again')
    # Read after the build, so the package's Version is the one cargo just compiled in.
    version=tomllib.loads((root/'Cargo.toml').read_text())['package']['version']
    (app/'VERSION').write_text(version+'\n')
    bin_dir=stage/'usr/bin';bin_dir.mkdir(parents=True);(bin_dir/'axentra-notepad').symlink_to('/opt/axentra/notepad/AxentraNotepad')
    desktop=stage/'usr/share/applications';desktop.mkdir(parents=True);shutil.copy2(root/'packaging/linux/axentra-notepad.desktop',desktop)
    subprocess.run(['desktop-file-validate',str(desktop/'axentra-notepad.desktop')],check=True)
    # Icons come from the suite renderer (assets/notepad-<size>.png).
    for size in (16,24,32,48,64,128,256,512):
        icon=stage/f'usr/share/icons/hicolor/{size}x{size}/apps/axentra-notepad.png';icon.parent.mkdir(parents=True)
        shutil.copy2(root/f'assets/notepad-{size}.png',icon)
    # Axentra Command reads packaged manifests from here.
    manifest=stage/'usr/share/axentra/command/apps';manifest.mkdir(parents=True);shutil.copy2(root/'packaging/linux/notepad.json',manifest)
    doc=stage/'usr/share/doc/axentra-notepad';doc.mkdir(parents=True)
    (doc/'copyright').write_text('Axentra Notepad\nCopyright Axentra contributors\nLicense: MIT (as declared by the source project)\n')
    size=installed_kib(stage)
    control=stage/'DEBIAN';control.mkdir()
    (control/'control').write_text(f'''Package: axentra-notepad
Version: {version}-{a.revision}
Section: editors
Priority: optional
Architecture: amd64
Maintainer: Axentra
Installed-Size: {size}
Depends: libc6 (>= 2.39), libgcc-s1, libglib2.0-0t64, libgtk-3-0t64, libgdk-pixbuf-2.0-0, libcairo2, libwebkit2gtk-4.1-0, libjavascriptcoregtk-4.1-0, libsoup-3.0-0, xdg-utils, fontconfig
Recommends: libglib2.0-bin
Description: Text and code editing with tabs, highlighting and find in files
 Axentra Notepad edits plain text and source code in tabs, with syntax
 highlighting, multiple cursors, find and replace across folders, encodings
 and line endings, and a session that keeps unsaved tabs. Native Rust
 application; built for Zorin OS 18 and Ubuntu 24.04 on x86_64.
''')
    # The stage root too: it's the data tarball's ./ entry, and TemporaryDirectory makes it 0700.
    for path in [stage,*stage.rglob('*')]:
        if path.is_symlink(): continue
        path.chmod(0o755 if path.is_dir() or path==app/EXE else 0o644)
    output=a.output/f'axentra-notepad_{version}-{a.revision}_amd64.deb'
    subprocess.run(['dpkg-deb','--root-owner-group','--build',str(stage),str(output)],check=True)
    write_sums(a.output,output)
    print(output)
