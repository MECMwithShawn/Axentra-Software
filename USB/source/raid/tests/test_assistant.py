import copy
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('assistant', Path(__file__).parents[1] / 'assistant.py')
a = importlib.util.module_from_spec(spec)
spec.loader.exec_module(a)
ROOT = '12345678-1234-1234-1234-123456789abc'
EFI = 'ABCD-1234'


def fixture():
    return {'live': True, 'uefi': True, 'arrays': {'/dev/md42': {'imsm': True, 'healthy': True}}, 'devices': {
        '/dev/md42': {'name': '/dev/md42', 'type': 'raid0', 'pttype': 'gpt'},
        '/dev/md42p5': {'name': '/dev/md42p5', 'type': 'part', 'parent': '/dev/md42', 'uuid': ROOT, 'fstype': 'ext4', 'parttype': a.LINUX, 'size': 80*1024**3, 'mountpoints': [None], 'ro': False},
        '/dev/md42p1': {'name': '/dev/md42p1', 'type': 'part', 'parent': '/dev/md42', 'uuid': EFI, 'fstype': 'vfat', 'parttype': a.ESP, 'size': 256*1024**2, 'mountpoints': [None], 'ro': False}}}


class PlanningTests(unittest.TestCase):
    def test_plan_uses_current_identities_and_preserves_efi(self):
        text = a.plan(fixture(), ROOT, EFI, 'dual')
        self.assertIn('/dev/md42p5', text)
        self.assertIn(ROOT, text)
        self.assertIn('Leave formatting OFF', text)
        self.assertIn('Continue Testing', text)
        self.assertIn('no permission to erase', a.plan(fixture(), ROOT, EFI, 'single'))

    def test_host_and_legacy_boot_blocked(self):
        for key in ('live', 'uefi'):
            snap = fixture()
            snap[key] = False
            with self.assertRaises(ValueError):
                a.select(snap, ROOT, EFI)

    def test_missing_degraded_or_non_imsm_array_blocked(self):
        for state in ({}, {'imsm': False, 'healthy': True}, {'imsm': True, 'healthy': False}):
            snap = fixture()
            snap['arrays']['/dev/md42'] = state
            with self.assertRaises(ValueError):
                a.select(snap, ROOT, EFI)

    def test_duplicate_or_missing_uuid_blocked(self):
        snap = fixture()
        snap['devices']['/dev/clone'] = copy.deepcopy(snap['devices']['/dev/md42p5'])
        with self.assertRaises(ValueError):
            a.select(snap, ROOT, EFI)
        for uuid in ('12345678-1234-1234-1234-123456789abd', '../../sda', '/dev/md42p5'):
            with self.assertRaises(ValueError):
                a.select(fixture(), uuid, EFI)

    def test_unsafe_root_choices_blocked(self):
        for field, value in [('fstype', 'ntfs'), ('parttype', a.ESP), ('type', 'disk'), ('size', 1024), ('ro', True), ('mountpoints', ['/']), ('parent', '/dev/nvme0n1')]:
            snap = fixture()
            snap['devices']['/dev/md42p5'][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                a.select(snap, ROOT, EFI)

    def test_wrong_esp_and_partition_table_blocked(self):
        for field, value in [('parent', '/dev/other'), ('parttype', a.LINUX), ('fstype', 'ext4')]:
            snap = fixture()
            snap['devices']['/dev/md42p1'][field] = value
            with self.assertRaises(ValueError):
                a.select(snap, ROOT, EFI)
        snap = fixture()
        snap['devices']['/dev/md42']['pttype'] = 'dos'
        with self.assertRaises(ValueError):
            a.select(snap, ROOT, EFI)


class FinalizationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.target = Path(self.tmp.name) / 'installed'
        for d in ('etc', 'dev', 'proc', 'sys', 'run', 'boot/efi'):
            (self.target / d).mkdir(parents=True, exist_ok=True)
        (self.target / 'etc/os-release').write_text('ID=zorin\n')
        self.marker = self.target / 'must-not-delete'
        self.marker.write_text('preserved')
        self.fstab = f'UUID={ROOT} /\nUUID={EFI} /boot/efi'
        def start(p):
            self.addCleanup(p.stop)
            return p.start()
        start(patch.object(a.os, 'geteuid', return_value=0))
        start(patch.object(a.os, 'readlink', side_effect=lambda p: 'host' if p == '/proc/1/ns/mnt' else 'private'))
        self.inventory = start(patch.object(a, 'inventory', return_value=fixture()))
        start(patch.object(a.tempfile, 'mkdtemp', return_value=str(self.target)))
        self.run = start(patch.object(a, 'run', side_effect=lambda *args: self.fstab))
        self.commands = start(patch.object(a.subprocess, 'run'))
        self.confirm = start(patch('builtins.input', return_value='PREPARE ' + ROOT))
        start(patch('builtins.print'))

    def test_success_efi_readonly_uuid_mounts_and_reverse_cleanup(self):
        a.finalize(ROOT, EFI, 'dual')
        calls = [c.args[0] for c in self.commands.call_args_list]
        self.assertIn(['mount', '-t', 'ext4', '-o', 'ro,noload', '/dev/disk/by-uuid/' + ROOT, str(self.target)], calls)
        self.assertIn(['mount', '-t', 'vfat', '-o', 'ro', '/dev/disk/by-uuid/' + EFI, str(self.target / 'boot/efi')], calls)
        self.assertIn([a.FINALIZER, '--target', str(self.target), '--mode', 'dual'], calls)
        self.assertEqual(calls[-1], ['umount', '--recursive', str(self.target)])
        self.assertTrue(self.marker.exists())

    def test_cancel_never_mounts(self):
        self.confirm.return_value = 'no'
        with self.assertRaises(ValueError):
            a.finalize(ROOT, EFI, 'dual')
        self.commands.assert_not_called()

    def test_stale_inventory_after_confirmation_blocks_mount(self):
        bad = fixture()
        bad['arrays']['/dev/md42']['healthy'] = False
        self.inventory.side_effect = [fixture(), fixture(), bad]
        with self.assertRaises(ValueError):
            a.finalize(ROOT, EFI, 'dual')
        self.commands.assert_not_called()

    def test_invalid_fstab_and_separate_boot_never_mount_rw(self):
        for fstab in ('UUID=wrong /', self.fstab + '\nUUID=other /boot', self.fstab + '\nUUID=other /'):
            self.fstab = fstab
            self.commands.reset_mock()
            with self.assertRaises(ValueError):
                a.finalize(ROOT, EFI, 'dual')
            calls = [c.args[0] for c in self.commands.call_args_list]
            self.assertEqual(sum(c[0] == 'mount' for c in calls), 1)
            self.assertTrue(self.marker.exists())

    def test_finalizer_failure_cleans_mounts_without_deleting_target(self):
        def fail(cmd, **kwargs):
            if cmd[0] == a.FINALIZER:
                raise a.subprocess.CalledProcessError(1, cmd)
        self.commands.side_effect = fail
        with self.assertRaises(a.subprocess.CalledProcessError):
            a.finalize(ROOT, EFI, 'dual')
        self.assertEqual(self.commands.call_args.args[0], ['umount', '--recursive', str(self.target)])
        self.assertTrue(self.marker.exists())

    def test_unmount_failure_never_recursively_deletes_target(self):
        def fail(cmd, **kwargs):
            if cmd[0] == 'umount':
                raise a.subprocess.CalledProcessError(1, cmd)
        self.commands.side_effect = fail
        with self.assertRaises(a.subprocess.CalledProcessError):
            a.finalize(ROOT, EFI, 'dual')
        self.assertEqual(self.marker.read_text(), 'preserved')

    def test_symlink_mountpoint_stops_before_rw_mount(self):
        (self.target / 'dev').rmdir()
        (self.target / 'dev').symlink_to('/dev')
        with self.assertRaises(ValueError):
            a.finalize(ROOT, EFI, 'dual')
        self.assertEqual(sum(c.args[0][0] == 'mount' for c in self.commands.call_args_list), 1)
