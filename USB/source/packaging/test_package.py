"""Regression checks for native Linux package dependency declarations."""
import tempfile
import unittest
from pathlib import Path

from package import build, linux_dependencies


class LinuxDependenciesTest(unittest.TestCase):
    def test_build_baselines_are_distinct(self):
        self.assertIn('libc6 (>= 2.35)', linux_dependencies('2.35'))
        self.assertIn('libc6 (>= 2.39)', linux_dependencies('2.39'))

    def test_missing_or_invalid_baseline_is_rejected(self):
        for baseline in (None, '', '2.34', '2.39\n', '2.39), fake-package'):
            with self.subTest(baseline=baseline), self.assertRaises(ValueError):
                linux_dependencies(baseline)

    def test_linux_packaging_requires_baseline_before_creating_outputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'output'
            with self.assertRaises(ValueError):
                build(None, Path(temporary) / 'binary', output)
            self.assertFalse(output.exists())


if __name__ == '__main__':
    unittest.main()
