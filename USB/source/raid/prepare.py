#!/usr/bin/env python3
"""Elevated image-only build worker. ISO bytes on stdout; diagnostics on stderr."""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time

# Do not install mdadm or bootloaders on the build host.
DEPENDENCIES = {
    'xorriso': 'xorriso', 'unsquashfs': 'squashfs-tools',
    'mksquashfs': 'squashfs-tools', 'unmkinitramfs': 'initramfs-tools-core',
    'lsinitramfs': 'initramfs-tools-core', 'cpio': 'cpio', 'gzip': 'gzip',
    'xz': 'xz-utils', 'zstd': 'zstd', 'unshare': 'util-linux',
}


def stage(message):
    print('AXENTRA: ' + message, file=sys.stderr, flush=True)


def open_owned(path, uid):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_uid != uid:
            raise ValueError('Input must be a regular file owned by the requesting user')
        return os.fdopen(fd, 'rb')
    except BaseException:
        os.close(fd)
        raise


def run(command, cancelled, cancellable=True):
    if cancelled():
        raise ValueError('RAID preparation cancelled')
    process = subprocess.Popen(command, stdout=sys.stderr, stderr=sys.stderr,
                               start_new_session=True)
    try:
        while process.poll() is None:
            if cancellable and cancelled():
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    pass
                # Also terminate descendants that survived the shell.
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait()
                raise ValueError('RAID preparation cancelled')
            time.sleep(0.2)
        if process.returncode:
            raise ValueError('Build command failed (exit %s)' % process.returncode)
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
    if cancelled():
        raise ValueError('RAID preparation cancelled')


def validate(request):
    if not isinstance(request, dict) or set(request) != {'source', 'sha256'}:
        raise ValueError('Invalid build request')
    if not isinstance(request['source'], str) or not Path(request['source']).is_absolute():
        raise ValueError('Source must be an absolute file path')
    if not isinstance(request['sha256'], str) or not re.fullmatch('[0-9a-fA-F]{64}', request['sha256']):
        raise ValueError('Publisher SHA-256 is required')


def main(request_path):
    if os.geteuid() != 0:
        raise ValueError('Administrator authentication is required')
    uid = int(os.environ.get('PKEXEC_UID', '-1'))
    if uid <= 0:
        raise ValueError('Launch preparation from the non-root desktop app')
    os.environ['PATH'] = '/usr/sbin:/usr/bin:/sbin:/bin'
    request_path = Path(request_path)
    with open_owned(request_path, uid) as f:
        request = json.loads(f.read(16385))
    validate(request)
    cancelled = lambda: (request_path.parent / 'cancel').exists()
    if shutil.disk_usage('/var/tmp').free < 80 * 1024**3:
        raise ValueError('RAID preparation requires at least 80 GiB free in /var/tmp')
    # Root-owned snapshot prevents replacing the source after verification.
    with tempfile.TemporaryDirectory(prefix='axentra-raid-', dir='/var/tmp') as tmp:
        tmp = Path(tmp)
        source = tmp / 'source.iso'
        stage('Copying and verifying the trusted source ISO')
        digest = hashlib.sha256()
        with open_owned(request['source'], uid) as src, source.open('xb') as dst:
            while chunk := src.read(4 * 1024**2):
                if cancelled():
                    raise ValueError('RAID preparation cancelled')
                digest.update(chunk)
                dst.write(chunk)
        if digest.hexdigest() != request['sha256'].lower():
            raise ValueError('Source checksum changed or does not match the publisher')
        packages = sorted({package for tool, package in DEPENDENCIES.items()
                           if shutil.which(tool) is None})
        if packages:
            if not shutil.which('apt-get'):
                raise ValueError('Install build dependencies on a Debian/Ubuntu/Zorin Linux host')
            stage('Installing missing build tools; Stop waits for package setup to finish')
            run(['apt-get', 'update'], cancelled, cancellable=False)
            run(['env', 'DEBIAN_FRONTEND=noninteractive', 'apt-get', 'install', '-y',
                 '--no-install-recommends', *packages], cancelled, cancellable=False)
        stage('Building the RAID image in a private mount namespace')
        output = tmp / 'prepared.iso'
        run(['bash', str(Path(__file__).with_name('build-iso.sh')), str(source),
             str(output), str(tmp / 'build')], cancelled)
        stage('Returning the prepared image for inspection')
        with output.open('rb') as f:
            while chunk := f.read(4 * 1024**2):
                if cancelled():
                    raise ValueError('RAID preparation cancelled')
                sys.stdout.buffer.write(chunk)
        sys.stdout.buffer.flush()


if __name__ == '__main__':
    try:
        main(sys.argv[1])
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        stage(str(error))
        sys.exit(1)
