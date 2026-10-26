import hashlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location("install_t3code", Path(__file__).with_name("install-t3code.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

FAKE_IMAGE = b"""#!/bin/sh
[ "$1" = --appimage-extract ] || exit 2
mkdir -p squashfs-root/resources
echo '#!/bin/sh' > squashfs-root/t3code
echo '#!/bin/sh' > squashfs-root/AppRun
: > squashfs-root/resources/app.asar
: > squashfs-root/t3code.png
printf '[Desktop Entry]\\nName=T3 Code\\nExec=AppRun %%U\\nIcon=t3code\\n' > squashfs-root/t3code.desktop
"""


def release(tag, published, assets=()):
    return {"tag_name": tag, "published_at": published, "assets": list(assets)}


class Response(io.BytesIO):
    def __enter__(self):
        return self

    def __exit__(self, *_):
        pass


class ReleaseChoiceTests(unittest.TestCase):
    releases = [
        release("v0.1.3-preview.20261026.3702", "2026-10-26T02:00:00Z"),
        release("v0.1.3-nightly.20261026.3710", "2026-10-26T04:00:00Z"),
        release("v0.1.3-nightly.20261026.3717", "2026-10-26T09:00:00Z"),
        release("v0.1.2", "2026-10-20T00:00:00Z"),
    ]

    def test_channel_is_read_from_the_tag(self):
        self.assertEqual(module.channel_of("v0.1.3-nightly.20261026.3717"), "nightly")
        self.assertEqual(module.channel_of("v0.1.3-preview.1"), "preview")
        self.assertEqual(module.channel_of("v0.1.2"), "stable")

    def test_newest_release_on_the_channel_wins(self):
        self.assertEqual(module.pick_release(self.releases, "nightly")["tag_name"], "v0.1.3-nightly.20261026.3717")
        self.assertEqual(module.pick_release(self.releases, "stable")["tag_name"], "v0.1.2")

    def test_exact_version_is_found_with_or_without_the_v(self):
        self.assertEqual(module.pick_release(self.releases, "nightly", "0.1.2")["tag_name"], "v0.1.2")
        with self.assertRaises(module.InstallError):
            module.pick_release(self.releases, "nightly", "9.9.9")

    def test_asset_matches_the_machine_and_unknown_machines_are_refused(self):
        found = release("v1", "x", [{"name": "T3-Code-1-arm64.AppImage"}, {"name": "T3-Code-1-x86_64.AppImage"}, {"name": "t3-1-linux-x64.tar.gz"}])
        self.assertEqual(module.pick_asset(found, "x86_64")["name"], "T3-Code-1-x86_64.AppImage")
        self.assertEqual(module.pick_asset(found, "aarch64")["name"], "T3-Code-1-arm64.AppImage")
        with self.assertRaises(module.InstallError):
            module.pick_asset(found, "riscv64")

    def test_checksums_come_from_both_sources_and_one_is_required(self):
        sums = module.parse_sums(f"{'a' * 64}  T3-Code-1-x86_64.AppImage\nnot a line\n")
        asset = {"name": "T3-Code-1-x86_64.AppImage", "digest": "sha256:" + "b" * 64}
        self.assertEqual(module.expected_digests(asset, sums), {"a" * 64, "b" * 64})
        with self.assertRaises(module.InstallError):
            module.expected_digests({"name": "other"}, {})

    def test_desktop_entry_gets_absolute_paths(self):
        text = "[Desktop Entry]\nExec=AppRun %U\nIcon=t3code\nName=T3\n"
        self.assertEqual(module.render_desktop_entry(text, Path("/opt/t3")), "[Desktop Entry]\nExec=/opt/t3/AppRun %U\nIcon=/opt/t3/t3code.png\nName=T3\n")


class InstallTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.addCleanup(self.directory.cleanup)
        self.digest = hashlib.sha256(FAKE_IMAGE).hexdigest()

    def args(self, **overrides):
        values = dict(prefix=str(self.root / "opt/t3code"), applications_dir=str(self.root / "applications"), channel="nightly", version=None, force=False, dry_run=False)
        return type("Args", (), {**values, **overrides})

    def fetcher(self, image=FAKE_IMAGE, digest=None):
        asset = {"name": "T3-Code-1-x86_64.AppImage", "size": len(image), "browser_download_url": "https://example/image", "digest": "sha256:" + (digest or self.digest)}
        listing = json.dumps([release("v1-nightly.1", "2026-10-26T00:00:00Z", [asset])]).encode()
        bodies = {"https://example/image": image}
        return lambda url, accept=None: Response(bodies.get(url, listing))

    def run_install(self, **kwargs):
        lines = []
        with patch.object(module, "fetch", self.fetcher(**{key: kwargs.pop(key) for key in ("image", "digest") if key in kwargs})), patch.object(module.platform, "machine", return_value="x86_64"):
            module.install(self.args(**kwargs), lines.append)
        return lines

    def test_fresh_install_places_the_app_entry_and_record(self):
        self.run_install()
        prefix = self.root / "opt/t3code"
        self.assertTrue((prefix / "t3code").exists())
        self.assertEqual(module.installed_tag(prefix), "v1-nightly.1")
        entry = (self.root / "applications/t3code.desktop").read_text()
        self.assertIn(f"Exec={prefix}/AppRun %U", entry)
        self.assertEqual([path.name for path in (self.root / "opt").iterdir()], ["t3code"])

    def test_nothing_changes_when_the_release_is_installed_and_force_reinstalls(self):
        self.run_install()
        marker = self.root / "opt/t3code" / module.MARKER
        marker.write_text(marker.read_text().replace("\n}", ',\n  "touched": true\n}'))
        self.assertIn("already installed", self.run_install()[0])
        self.assertIn("touched", marker.read_text())
        self.run_install(force=True)
        self.assertNotIn("touched", marker.read_text())

    def test_a_wrong_checksum_installs_nothing_and_keeps_the_old_app(self):
        prefix = self.root / "opt/t3code"
        prefix.mkdir(parents=True)
        (prefix / "kept").write_text("old")
        with self.assertRaises(module.InstallError):
            self.run_install(digest="0" * 64)
        self.assertEqual((prefix / "kept").read_text(), "old")
        self.assertEqual([path.name for path in (self.root / "opt").iterdir()], ["t3code"])
        self.assertFalse((self.root / "applications").exists())

    def test_an_app_by_hand_is_replaced_and_dry_run_changes_nothing(self):
        prefix = self.root / "opt/t3code"
        prefix.mkdir(parents=True)
        (prefix / "kept").write_text("old")
        self.run_install(dry_run=True)
        self.assertTrue((prefix / "kept").exists())
        self.run_install()
        self.assertFalse((prefix / "kept").exists())
        self.assertTrue((prefix / "t3code").exists())


if __name__ == "__main__":
    unittest.main()
