import importlib.util
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).parents[1] / "bump_patch_version.py"
SPEC = importlib.util.spec_from_file_location("bump_patch_version", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
bump_patch_version = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bump_patch_version)


class BumpPatchVersionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        directory = Path(self.temporary_directory.name)
        self.manifest = directory / "Cargo.toml"
        self.lockfile = directory / "Cargo.lock"

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def write_versions(self, manifest_version: str, lockfile_version: str) -> None:
        self.manifest.write_text(
            f'[package]\nname = "astesia"\nversion = "{manifest_version}"\n',
            encoding="utf-8",
        )
        self.lockfile.write_text(
            "[[package]]\nname = \"dependency\"\nversion = \"9.0.0\"\n\n"
            f'[[package]]\nname = "astesia"\nversion = "{lockfile_version}"\n',
            encoding="utf-8",
        )

    def test_bumps_matching_package_and_lockfile_versions(self) -> None:
        self.write_versions("1.2.3", "1.2.3")

        version = bump_patch_version.bump_versions(self.manifest, self.lockfile)

        self.assertEqual(version, "1.2.4")
        self.assertIn('version = "1.2.4"', self.manifest.read_text(encoding="utf-8"))
        lockfile = self.lockfile.read_text(encoding="utf-8")
        self.assertIn('name = "dependency"\nversion = "9.0.0"', lockfile)
        self.assertIn('name = "astesia"\nversion = "1.2.4"', lockfile)

    def test_prerelease_bumps_to_the_next_stable_patch(self) -> None:
        self.write_versions("2.0.0-rc.3", "2.0.0-rc.3")

        self.assertEqual(
            bump_patch_version.bump_versions(self.manifest, self.lockfile), "2.0.1"
        )

    def test_mismatch_leaves_both_files_unchanged(self) -> None:
        self.write_versions("1.2.3", "1.2.2")
        before = (self.manifest.read_bytes(), self.lockfile.read_bytes())

        with self.assertRaisesRegex(ValueError, "version mismatch"):
            bump_patch_version.bump_versions(self.manifest, self.lockfile)

        self.assertEqual(before, (self.manifest.read_bytes(), self.lockfile.read_bytes()))


if __name__ == "__main__":
    unittest.main()
