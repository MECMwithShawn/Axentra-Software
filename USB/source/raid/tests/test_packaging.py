import importlib.util
import io
from pathlib import Path
import tarfile
import unittest

spec = importlib.util.spec_from_file_location('package', Path(__file__).parents[2] / 'packaging/package.py')
p = importlib.util.module_from_spec(spec)
spec.loader.exec_module(p)


class PackagingTests(unittest.TestCase):
    def test_parent_directories_precede_files_for_clean_dpkg_install(self):
        data = p.tar([('usr/share/doc/axentra-usb/README.md', b'doc', 0o644),
                      ('usr/share/doc/axentra-usb/LICENSE', b'license', 0o644),
                      ('usr/bin/axentra-usb', b'app', 0o755)])
        with tarfile.open(fileobj=io.BytesIO(data), mode='r:gz') as archive:
            members = archive.getmembers()
            names = [m.name for m in members]
            self.assertEqual(len(names), len(set(names)))
            for member in members:
                for parent in Path(member.name).parents:
                    if str(parent) == '.':
                        continue
                    self.assertLess(names.index(str(parent)), names.index(member.name))
                    self.assertTrue(archive.getmember(str(parent)).isdir())
            self.assertEqual(archive.getmember('usr/bin/axentra-usb').mode, 0o755)
            self.assertEqual(archive.extractfile('usr/share/doc/axentra-usb/README.md').read(), b'doc')
