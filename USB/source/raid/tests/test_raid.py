import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('raid', Path(__file__).parents[1] / 'axentra_raid.py')
raid = importlib.util.module_from_spec(spec)
spec.loader.exec_module(raid)
C = '11111111:22222222:33333333:44444444'
V = 'aaaaaaaa:bbbbbbbb:cccccccc:dddddddd'
SCAN = f'ARRAY /dev/md127 metadata=imsm UUID={C}\nARRAY /dev/md126 container=/dev/md127 member=0 UUID={V}\n'
FILES = 'sbin/mdadm\nsbin/mdmon\netc/mdadm/mdadm.conf\nscripts/local-top/axentra-imsm\nusr/lib/udev/rules.d/64-md-raid-assembly.rules\n'


class ConfigTests(unittest.TestCase):
    def test_volatile_names_replaced_and_container_resolved(self):
        conf = raid.array_lines(SCAN, V)
        self.assertNotIn('/dev/md126', conf)
        self.assertNotIn('/dev/md127', conf)
        self.assertIn('container=' + C, conf)
        self.assertEqual(conf, raid.array_lines(SCAN.replace('md127', 'md119').replace('md126', 'md118'), V))

    def test_invalid_scans_refused(self):
        for scan in ('', f'ARRAY /dev/md1 UUID={V}', SCAN.replace(C, 'bad'), SCAN.splitlines()[1], SCAN.replace('member=0', 'member=x')):
            with self.subTest(scan=scan), self.assertRaises(ValueError):
                raid.array_lines(scan)
        with self.assertRaises(ValueError):
            raid.array_lines(SCAN, C)

    def test_config_idempotence_and_preservation(self):
        generated = raid.array_lines(SCAN)
        old = f'MAILADDR admin@example.test\nAUTO -imsm +1.x -all\nARRAY /dev/md126 UUID={V}\nARRAY /dev/md9 UUID=99999999:99999999:99999999:99999999\n'
        result = raid.merge_config(old, generated)
        self.assertIn('AUTO +imsm +1.x -all', result)
        self.assertIn('MAILADDR admin@example.test', result)
        self.assertIn('ARRAY /dev/md9', result)
        self.assertEqual(result, raid.merge_config(result, generated))
        self.assertEqual(result.count('UUID=' + V), 1)

    def test_corrupt_block_refused(self):
        with self.assertRaises(ValueError):
            raid.merge_config(raid.BEGIN, raid.array_lines(SCAN))

    def test_missing_mdmon_refused(self):
        raid.verify_initrd(FILES)
        with self.assertRaises(ValueError):
            raid.verify_initrd(FILES.replace('sbin/mdmon\n', ''))

    def test_selected_volume_excludes_unrelated_container(self):
        other = '99999999:99999999:99999999:99999999'
        self.assertNotIn(other, raid.array_lines(SCAN + f'ARRAY /dev/md120 metadata=imsm UUID={other}\n', V))

    def test_uppercase_uuid_replaced(self):
        result = raid.merge_config(f'ARRAY /dev/md8 UUID={V.upper()}\n', raid.array_lines(SCAN))
        self.assertNotIn('/dev/md8', result)

    def test_unpacked_config_and_builtin_driver(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            config = root / 'etc/mdadm/mdadm.conf'
            config.parent.mkdir(parents=True)
            config.write_text('expected')
            (root / 'modules.builtin').write_text('kernel/drivers/md/raid0.ko\n')
            with patch.object(raid.tempfile, 'TemporaryDirectory') as directory, patch.object(raid, 'run'):
                directory.return_value.__enter__.return_value = tmp
                raid.verify_config(Path('fixture'), 'expected')
                with self.assertRaisesRegex(ValueError, 'persistent UUID'):
                    raid.verify_config(Path('fixture'), 'stale')
                (root / 'modules.builtin').write_text('')
                with self.assertRaisesRegex(ValueError, 'RAID 0 driver'):
                    raid.verify_config(Path('fixture'), 'expected')


class FinalizeTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / 'target'
        for name, text in {
            'etc/os-release': 'ID=zorin\n', 'etc/fstab': '',
            'etc/mdadm/mdadm.conf': '# original\n',
            'etc/initramfs-tools/hooks/zz-axentra-imsm': 'hook',
            'usr/sbin/mdadm': '', 'usr/sbin/mdmon': '',
            'boot/efi/EFI/Microsoft/Boot/bootmgfw.efi': 'windows',
            'boot/efi/EFI/ubuntu/shimx64.efi': 'shim',
            'boot/initrd.img-fixture': 'fixture',
        }.items():
            p = self.root / name
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(text)
        self.calls = []
        self.fstab = 'UUID=root-id /\nUUID=ABCD-1234 /boot/efi'
        self.fs = 'vfat'
        self.level = 'raid0'
        self.confirm = 'FINALIZE root-id'
        self.bad_initrd = False
        self.fail_update = False
        self.change_efi = False
        real_read = Path.read_text
        real_exists = Path.exists
        def read(path, *a, **kw):
            if str(path).startswith('/sys/class/block/md'):
                return {'level': self.level, 'metadata_version': 'external:imsm' if 'md127/' in str(path) else 'external:/md127/0'}[path.name]
            return real_read(path, *a, **kw)
        self.enter(patch.object(Path, 'read_text', read))
        self.enter(patch.object(Path, 'exists', lambda p: True if str(p) == '/sys/firmware/efi' else real_exists(p)))
        self.enter(patch.object(raid.os, 'geteuid', return_value=0))
        self.enter(patch.object(raid, 'mounted', self.mounted))
        self.enter(patch.object(raid, 'run', self.fake_run))
        self.enter(patch('builtins.input', lambda _: self.confirm))
        self.enter(patch('builtins.print'))

    def enter(self, p):
        self.addCleanup(p.stop)
        return p.start()

    def mounted(self, target, field):
        esp = str(target).endswith('/boot/efi')
        return {'SOURCE': '/dev/md116p3' if not esp else '/dev/md116p1', 'FSTYPE': self.fs if esp else 'ext4', 'UUID': 'ABCD-1234' if esp else 'root-id', 'TARGET': str(target)}[field]

    def fake_run(self, *cmd):
        self.calls.append(cmd)
        if cmd[0] == 'lsblk':
            return 'md116' if 'PKNAME' in cmd else 'c12a7328-f81f-11d2-ba4b-00a0c93ec93b'
        if cmd[0] == 'findmnt':
            return self.fstab
        if cmd[0] == 'mdadm':
            return 'MD_UUID=' + V if '--export' in cmd else SCAN
        if cmd[0] == 'unmkinitramfs':
            out = Path(cmd[2])
            (out / 'etc/mdadm').mkdir(parents=True)
            (out / 'etc/mdadm/mdadm.conf').write_text((self.root / 'etc/mdadm/mdadm.conf').read_text())
            (out / 'raid0.ko.zst').touch()
            return ''
        if cmd[0] == 'lsinitramfs':
            return '' if self.bad_initrd else FILES
        if 'grub-probe' in cmd:
            return 'root-id'
        if 'dpkg-query' in cmd:
            return 'install ok installed'
        if 'update-initramfs' in cmd and self.fail_update:
            raise raid.subprocess.CalledProcessError(1, cmd)
        if 'update-grub' in cmd and self.change_efi:
            (self.root / 'boot/efi/EFI/Microsoft/Boot/bootmgfw.efi').write_text('changed')
        return ''

    def assert_no_mutation(self):
        self.assertFalse(any('update-initramfs' in c or 'update-grub' in c for c in self.calls))
        self.assertEqual((self.root / 'etc/mdadm/mdadm.conf').read_text(), '# original\n')

    def test_dual_success_and_repeat(self):
        before = raid.efi_hashes(self.root / 'boot/efi')
        raid.finalize(self.root, 'dual')
        conf = (self.root / 'etc/mdadm/mdadm.conf').read_text()
        raid.finalize(self.root, 'dual')
        self.assertEqual(conf, (self.root / 'etc/mdadm/mdadm.conf').read_text())
        self.assertEqual(before, raid.efi_hashes(self.root / 'boot/efi'))
        self.assertIn('ABCD-1234', (self.root / 'etc/grub.d/41_axentra_windows').read_text())
        self.assertTrue(any('update-initramfs' in c for c in self.calls))
        self.assertTrue(any('grub-script-check' in c for c in self.calls))

    def test_single_without_windows(self):
        (self.root / 'boot/efi/EFI/Microsoft/Boot/bootmgfw.efi').unlink()
        raid.finalize(self.root, 'single')
        self.assertFalse((self.root / 'etc/grub.d/41_axentra_windows').exists())

    def test_dual_without_windows_refused(self):
        (self.root / 'boot/efi/EFI/Microsoft/Boot/bootmgfw.efi').unlink()
        with self.assertRaises(ValueError):
            raid.finalize(self.root, 'dual')
        self.assert_no_mutation()

    def test_non_uuid_fstab_refused(self):
        self.fstab = '/dev/md116p3 /\nUUID=ABCD-1234 /boot/efi'
        with self.assertRaises(ValueError):
            raid.finalize(self.root, 'dual')
        self.assert_no_mutation()

    def test_wrong_esp_refused(self):
        self.fs = 'ntfs'
        with self.assertRaises(ValueError):
            raid.finalize(self.root, 'dual')
        self.assert_no_mutation()

    def test_non_raid0_refused(self):
        self.level = 'raid1'
        with self.assertRaises(ValueError):
            raid.finalize(self.root, 'dual')
        self.assert_no_mutation()

    def test_cancel_refused(self):
        self.confirm = 'yes'
        with self.assertRaises(ValueError):
            raid.finalize(self.root, 'dual')
        self.assert_no_mutation()

    def test_bad_initrd_prevents_grub_and_success(self):
        self.bad_initrd = True
        with self.assertRaises(ValueError):
            raid.finalize(self.root, 'dual')
        self.assertFalse(any('update-grub' in c for c in self.calls))

    def test_initramfs_command_failure_stops_finalization(self):
        self.fail_update = True
        with self.assertRaises(raid.subprocess.CalledProcessError):
            raid.finalize(self.root, 'dual')
        self.assertFalse(any('update-grub' in c for c in self.calls))

    def test_efi_mutation_is_reported_as_failure(self):
        self.change_efi = True
        with self.assertRaisesRegex(ValueError, 'EFI files changed'):
            raid.finalize(self.root, 'dual')

    def test_missing_esp_fstab_entry_refused(self):
        self.fstab = 'UUID=root-id /'
        with self.assertRaises(ValueError):
            raid.finalize(self.root, 'dual')
        self.assert_no_mutation()


if __name__ == '__main__':
    unittest.main()
