#!/usr/bin/env python3
"""Run install-dojo-dm.sh into a temporary root, with no sudo and no devenv."""

import os
import shutil
import stat
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
REPO = SCRIPTS.parent
STORE = "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
NIRI_LIBS = f"{STORE}-mesa/lib:{STORE}-wayland/lib"
DESKTOP_LIBS = f"{STORE}-mesa/lib:{STORE}-pipewire/lib"
TUNABLES = (
    'case ":${GLIBC_TUNABLES:-}:" in\n'
    "    *:glibc.malloc.mmap_threshold=*:*) ;;\n"
    '    *) export GLIBC_TUNABLES="${GLIBC_TUNABLES:+$GLIBC_TUNABLES:}glibc.malloc.mmap_threshold=131072" ;;\n'
    "esac\n"
)

FAKE_LDD = f"""#!/bin/sh
grep -q UNRESOLVED "$1" && echo "libgone.so => not found"
echo "	libfake.so => {STORE}-fake/lib/libfake.so (0x0)"
exit 0
"""
FAKE_NIRI = """#!/bin/sh
# fake niri
[ "$1" = validate ] && exit 0
exit 0
"""
FAKE_CHECK = """#!/bin/sh
grep -q BROKEN "$1" && { echo "dojo-dm-pam cannot load libpam" >&2; exit 1; }
exit 0
"""
FAKE_NIX_STORE = '#!/bin/sh\nln -sfn "$5" "$2"\n'


def write(path, text, mode=0o755):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    path.chmod(mode)


class Fixture:
    def __init__(self, base):
        self.base = Path(base)
        self.root = self.base / "root"
        self.scripts = self.base / "repo/scripts"
        self.scripts.mkdir(parents=True)
        for name in ("install-dojo-dm.sh", "prune-installations.sh"):
            shutil.copy2(SCRIPTS / name, self.scripts / name)
        write(self.scripts / "check-dojo-lock-helper.py", FAKE_CHECK)
        kdl = self.base / "repo/system-config/dojo-dm"
        kdl.mkdir(parents=True)
        shutil.copy2(REPO / "system-config/dojo-dm/greeter.kdl", kdl / "greeter.kdl")
        self.bin = self.base / "bin"
        self.commands = self.base / "commands"
        write(self.commands / "ldd", FAKE_LDD)
        write(self.commands / "nix-store", FAKE_NIX_STORE)
        self.binaries(dm="dm v1\n", pam="pam v1\n", greeter="greeter v1\n")
        self.niri_installation = self.base / "data/dojo/niri/rev1"
        self.desktop_installation = self.base / "data/dojo/desktop/rev1"
        write(self.niri_installation / "niri", FAKE_NIRI)
        self.environments(NIRI_LIBS, DESKTOP_LIBS)
        for kind in ("niri", "desktop"):
            (self.base / f"data/dojo/{kind}/current").symlink_to("rev1")

    def binaries(self, dm, pam, greeter):
        write(self.bin / "dojo-dm", dm)
        write(self.bin / "dojo-dm-pam", pam)
        write(self.bin / "dojo-greeter", greeter)

    def environments(self, niri_libs, desktop_libs):
        write(
            self.niri_installation / "environment.sh",
            f"export LD_LIBRARY_PATH={niri_libs}\nexport DOJO_SEMANTIC_AGENT_EXE=/nix/store/x-agent\n",
            0o644,
        )
        write(
            self.desktop_installation / "environment.sh",
            f"export LD_LIBRARY_PATH={desktop_libs}\n{TUNABLES}",
            0o644,
        )

    @property
    def lib(self):
        return self.root / "usr/local/lib/dojo-dm"

    def run(self, *arguments, **extra):
        environment = {
            **os.environ,
            "HOME": str(self.base / "home"),
            "XDG_DATA_HOME": str(self.base / "data"),
            "DOJO_DM_ROOT": str(self.root),
            "PATH": f"{self.commands}:{os.environ['PATH']}",
            "TMPDIR": str(self.base),
            **extra,
        }
        for name in ("LD_LIBRARY_PATH", "GLIBC_TUNABLES"):
            environment.pop(name, None)
        return subprocess.run(
            ["bash", str(self.scripts / "install-dojo-dm.sh"), str(self.bin), *arguments],
            env=environment,
            capture_output=True,
            text=True,
        )

    def release(self):
        return (self.lib / "current").resolve()


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="dojo-dm-installer-test-")
        self.addCleanup(self.temporary.cleanup)
        self.fixture = Fixture(self.temporary.name)

    def test_installs_real_copies_with_a_store_only_environment(self):
        result = self.fixture.run()
        self.assertEqual(result.returncode, 0, result.stderr)
        release = self.fixture.release()
        self.assertEqual(release.parent, self.fixture.lib.resolve())
        self.assertEqual(
            sorted(path.name for path in release.iterdir()),
            ["dojo-dm", "dojo-dm-pam", "dojo-greeter", "environment.sh", "greeter.kdl", "niri", "roots", "run"],
        )
        for name in ("dojo-dm", "dojo-dm-pam", "dojo-greeter", "niri", "run"):
            self.assertTrue(os.access(release / name, os.X_OK), name)
            self.assertFalse((release / name).is_symlink(), name)
        self.assertEqual((release / "dojo-dm").read_text(), "dm v1\n")
        self.assertFalse(stat.S_IMODE((release / "environment.sh").stat().st_mode) & 0o111)
        environment = (release / "environment.sh").read_text()
        self.assertEqual(
            environment.splitlines()[0],
            f"export LD_LIBRARY_PATH={STORE}-mesa/lib:{STORE}-wayland/lib:{STORE}-pipewire/lib",
        )
        self.assertIn("export GLIBC_TUNABLES=glibc.malloc.mmap_threshold=131072", environment)
        self.assertNotIn("DOJO_SEMANTIC_AGENT_EXE", environment)
        for name in ("run", "environment.sh", "greeter.kdl"):
            self.assertNotIn("devenv", (release / name).read_text())
        self.assertEqual(
            sorted(path.name for path in (release / "roots").iterdir()),
            sorted(f"{STORE.rsplit('/', 1)[1]}-{suffix}" for suffix in ("fake", "mesa", "pipewire", "wayland")),
        )

    def test_the_vulkan_driver_is_pinned_like_the_locker(self):
        icds = Path(self.temporary.name) / "icd.d"
        icds.mkdir()

        def pinned(*names):
            for name in names:
                (icds / name).write_text("{}")
            result = self.fixture.run(DOJO_DM_ICD_DIR=str(icds))
            self.assertEqual(result.returncode, 0, result.stderr)
            return [
                line
                for line in (self.fixture.release() / "environment.sh").read_text().splitlines()
                if line.startswith("export VK_DRIVER_FILES=")
            ]

        self.assertEqual(pinned(), [])
        self.assertEqual(
            pinned("radeon_icd.x86_64.json"), [f"export VK_DRIVER_FILES={icds}/radeon_icd.x86_64.json"]
        )
        self.assertEqual(
            pinned("nvidia_icd.json"), [f"export VK_DRIVER_FILES={icds}/nvidia_icd.json"]
        )

    def test_greeter_config_names_this_release(self):
        self.assertEqual(self.fixture.run().returncode, 0)
        release = self.fixture.release()
        text = (release / "greeter.kdl").read_text()
        self.assertNotIn("@RELEASE@", text)
        self.assertNotIn("@STATE@", text)
        # The monitor layout the display manager writes is included, and may be absent.
        self.assertIn('include "/var/lib/dojo-dm/greeter-outputs.kdl" optional=true', text)
        self.assertIn(
            f'spawn-sh-at-startup ". /usr/local/lib/dojo-dm/{release.name}/environment.sh'
            f' && /usr/local/lib/dojo-dm/{release.name}/dojo-greeter;'
            f' /usr/local/lib/dojo-dm/{release.name}/niri msg action quit --skip-confirmation"',
            text,
        )

    def test_wrapper_runs_dojo_dm_with_the_release_environment(self):
        self.assertEqual(self.fixture.run().returncode, 0)
        release = self.fixture.release()
        write(release / "dojo-dm", '#!/bin/sh\necho "$LD_LIBRARY_PATH"\n')
        output = subprocess.run(
            [str(self.fixture.lib / "current/run")], capture_output=True, text=True, env={"PATH": "/usr/bin"}
        )
        self.assertEqual(output.returncode, 0, output.stderr)
        self.assertTrue(output.stdout.startswith(NIRI_LIBS), output.stdout)

    def test_second_run_changes_nothing(self):
        self.assertEqual(self.fixture.run().returncode, 0)
        release = self.fixture.release()
        before = {path: path.stat().st_mtime_ns for path in release.iterdir() if path.name != "roots"}
        time.sleep(0.01)
        result = self.fixture.run()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("already installed", result.stdout)
        self.assertEqual(release, self.fixture.release())
        self.assertEqual(before, {path: path.stat().st_mtime_ns for path in release.iterdir() if path.name != "roots"})
        self.assertEqual(sorted(path.name for path in self.fixture.lib.iterdir()), sorted(["current", release.name]))

    def test_new_binaries_make_a_new_release_and_keep_the_old_one_reachable(self):
        self.assertEqual(self.fixture.run().returncode, 0)
        first = self.fixture.release()
        self.fixture.binaries(dm="dm v2\n", pam="pam v1\n", greeter="greeter v1\n")
        result = self.fixture.run()
        self.assertEqual(result.returncode, 0, result.stderr)
        second = self.fixture.release()
        self.assertNotEqual(first, second)
        self.assertEqual((self.fixture.lib / "previous").resolve(), first)
        self.assertEqual((second / "dojo-dm").read_text(), "dm v2\n")
        self.assertEqual((first / "dojo-dm").read_text(), "dm v1\n")

    def test_changed_library_path_makes_a_new_release(self):
        self.assertEqual(self.fixture.run().returncode, 0)
        first = self.fixture.release()
        self.fixture.environments(NIRI_LIBS, f"{STORE}-pipewire2/lib")
        self.assertEqual(self.fixture.run().returncode, 0)
        self.assertNotEqual(first, self.fixture.release())

    def test_a_modified_release_is_backed_up_and_rebuilt(self):
        self.assertEqual(self.fixture.run().returncode, 0)
        release = self.fixture.release()
        (release / "dojo-greeter").write_text("tampered\n")
        result = self.fixture.run()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Backed up existing release", result.stdout)
        self.assertEqual((release / "dojo-greeter").read_text(), "greeter v1\n")
        backups = [path for path in self.fixture.lib.iterdir() if ".backup-" in path.name]
        self.assertEqual(len(backups), 1)
        self.assertEqual((backups[0] / "dojo-greeter").read_text(), "tampered\n")

    def test_a_current_that_is_not_a_link_is_backed_up(self):
        (self.fixture.lib / "current").mkdir(parents=True)
        (self.fixture.lib / "current/old").write_text("old\n")
        result = self.fixture.run()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.fixture.lib / "current").is_symlink())
        backups = [path for path in self.fixture.lib.iterdir() if path.name.startswith("current.backup-")]
        self.assertEqual(len(backups), 1)
        self.assertEqual((backups[0] / "old").read_text(), "old\n")

    def test_old_releases_are_pruned_to_four(self):
        releases = []
        for version in range(1, 7):
            self.fixture.binaries(dm=f"dm v{version}\n", pam="pam\n", greeter="greeter\n")
            self.assertEqual(self.fixture.run().returncode, 0)
            releases.append(self.fixture.release())
            time.sleep(0.02)
        remaining = {path.name for path in self.fixture.lib.iterdir() if path.is_dir() and not path.is_symlink()}
        self.assertEqual(len(remaining), 4)
        self.assertIn(releases[-1].name, remaining)
        self.assertNotIn(releases[0].name, remaining)

    def refuses(self, result, message):
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(message, result.stderr)
        self.assertFalse((self.fixture.root / "usr").exists(), "nothing may be installed")

    def test_refuses_when_the_pam_helper_check_fails(self):
        self.fixture.binaries(dm="dm\n", pam="BROKEN\n", greeter="greeter\n")
        self.refuses(self.fixture.run(), "PAM helper does not work")

    def test_refuses_unresolved_libraries(self):
        self.fixture.binaries(dm="dm\n", pam="pam\n", greeter="UNRESOLVED\n")
        self.refuses(self.fixture.run(), "dojo-greeter has unresolved runtime libraries")

    def test_refuses_a_library_path_outside_the_store(self):
        self.fixture.environments(f"{NIRI_LIBS}:/home/user/lib", DESKTOP_LIBS)
        self.refuses(self.fixture.run(), "only /nix/store paths are allowed")

    def test_refuses_without_the_niri_fork_or_missing_binaries(self):
        (self.fixture.base / "data/dojo/niri/current").unlink()
        self.refuses(self.fixture.run(), "Niri fork is not installed")
        (self.fixture.bin / "dojo-greeter").unlink()
        self.refuses(self.fixture.run(), "Missing")

    def test_refuses_a_symlinked_installation_directory(self):
        self.fixture.lib.parent.mkdir(parents=True)
        self.fixture.lib.symlink_to(self.fixture.base)
        result = self.fixture.run()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unsafe installation directory", result.stderr)

    def test_leaves_no_staging_files_behind(self):
        self.assertEqual(self.fixture.run().returncode, 0)
        self.assertEqual([path.name for path in self.fixture.base.glob("tmp*")], [])
        self.assertEqual([path for path in self.fixture.lib.iterdir() if ".new-" in path.name], [])


if __name__ == "__main__":
    unittest.main()
