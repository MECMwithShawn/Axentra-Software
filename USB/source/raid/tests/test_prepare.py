import io
import hashlib
import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('prepare', Path(__file__).parents[1] / 'prepare.py')
p = importlib.util.module_from_spec(spec)
spec.loader.exec_module(p)


class PrepareTests(unittest.TestCase):
    def test_request_rejects_invalid_checksum_paths_and_extra_commands(self):
        p.validate({'source': '/tmp/image.iso', 'sha256': 'a' * 64})
        for value in [None, {}, {'source': 'relative.iso', 'sha256': 'a'*64},
                      {'source': '/tmp/a', 'sha256': 'bad'},
                      {'source': '/tmp/a', 'sha256': 'a'*64, 'command': 'anything'}]:
            with self.assertRaises(ValueError):
                p.validate(value)

    def test_only_regular_owned_inputs(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'input'
            path.write_bytes(b'test')
            with p.open_owned(path, os.getuid()) as f:
                self.assertEqual(f.read(), b'test')
            with self.assertRaises(ValueError):
                p.open_owned(path, os.getuid()+1)
            link = Path(tmp) / 'link'
            link.symlink_to(path)
            with self.assertRaises(OSError):
                p.open_owned(link, os.getuid())
            fifo = Path(tmp) / 'fifo'
            os.mkfifo(fifo)
            with self.assertRaises(ValueError):
                p.open_owned(fifo, os.getuid())

    def test_cancel_before_command_never_launches_it(self):
        with patch.object(p.subprocess, 'Popen') as spawn:
            with self.assertRaisesRegex(ValueError, 'cancelled'):
                p.run(['unused'], lambda: True)
            spawn.assert_not_called()

    def test_dependency_list_excludes_host_raid_and_bootloader_installation(self):
        self.assertNotIn('mdadm', p.DEPENDENCIES.values())
        self.assertFalse(any('grub' in x or 'shim' in x for x in p.DEPENDENCIES.values()))
        self.assertEqual(p.DEPENDENCIES['xorriso'], 'xorriso')

    def test_bad_source_hash_stops_before_dependency_install_or_build(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / 'source.iso'
            source.write_bytes(b'ISO fixture')
            request = root / 'request.json'
            request.write_text(p.json.dumps({'source': str(source), 'sha256': 'a'*64}))
            with patch.object(p.os, 'geteuid', return_value=0), patch.dict(p.os.environ, {'PKEXEC_UID': str(os.getuid())}), patch.object(p.shutil, 'disk_usage') as usage, patch.object(p, 'run') as run:
                usage.return_value.free = 100 * 1024**3
                with self.assertRaisesRegex(ValueError, 'checksum'):
                    p.main(request)
                run.assert_not_called()

    def test_build_reads_verified_snapshot_and_returns_only_result_bytes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / 'source.iso'
            source.write_bytes(b'ISO fixture')
            request = root / 'request.json'
            request.write_text(p.json.dumps({'source': str(source), 'sha256': hashlib.sha256(b'ISO fixture').hexdigest()}))
            output = io.BytesIO()
            calls = []
            def build(command, cancelled, **kwargs):
                calls.append(command)
                self.assertEqual(command[0], 'bash')
                snapshot = Path(command[2])
                self.assertNotEqual(snapshot, source)
                self.assertEqual(snapshot.read_bytes(), b'ISO fixture')
                Path(command[3]).write_bytes(b'prepared ISO')
            with patch.object(p.os, 'geteuid', return_value=0), patch.dict(p.os.environ, {'PKEXEC_UID': str(os.getuid())}), patch.object(p.shutil, 'disk_usage') as usage, patch.object(p.shutil, 'which', return_value='/usr/bin/tool'), patch.object(p, 'run', side_effect=build), patch.object(p.sys, 'stdout') as stdout:
                usage.return_value.free = 100 * 1024**3
                stdout.buffer = output
                p.main(request)
            self.assertEqual(output.getvalue(), b'prepared ISO')
            self.assertEqual(len(calls), 1)
            self.assertEqual(source.read_bytes(), b'ISO fixture')

    def test_cancel_during_build_terminates_its_process_group(self):
        with patch.object(p.subprocess, 'Popen') as spawn, patch.object(p.os, 'killpg') as kill:
            process = spawn.return_value
            process.pid = 12345
            process.poll.side_effect = [None, 0]
            calls = iter([False, True])
            with self.assertRaisesRegex(ValueError, 'cancelled'):
                p.run(['unused'], lambda: next(calls))
            kill.assert_any_call(12345, p.signal.SIGTERM)
            self.assertTrue(spawn.call_args.kwargs['start_new_session'])
