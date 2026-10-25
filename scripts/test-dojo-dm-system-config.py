#!/usr/bin/env python3
"""Check the tracked login screen files against the installer and each other."""

import importlib.util
import re
import shlex
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
CONFIG = REPO / "system-config"

spec = importlib.util.spec_from_file_location("inventory", Path(__file__).with_name("capture-privileged-config.py"))
inventory = importlib.util.module_from_spec(spec)
spec.loader.exec_module(inventory)


def stack_lines(name):
    """PAM lines with comments dropped, as (type, rest) pairs."""
    lines = []
    for line in (CONFIG / "pam.d" / name).read_text().splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            kind, _, rest = line.partition(" ")
            lines.append((kind.lstrip("-"), " ".join(rest.split())))
    return lines


def unit_section(name, section):
    text = (CONFIG / "systemd/system" / name).read_text()
    body = re.search(rf"^\[{section}\]\n(.*?)(?=^\[|\Z)", text, re.M | re.S).group(1)
    pairs = {}
    for line in body.splitlines():
        if line and not line.startswith("#"):
            key, _, value = line.partition("=")
            pairs.setdefault(key, []).append(value)
    return pairs


class PolkitFingerprint(unittest.TestCase):
    def test_polkit_asks_a_finger_then_the_password_and_never_denies_everything(self):
        lines = stack_lines("polkit-1")
        self.assertEqual(lines[0], ("auth", "sufficient pam_fprintd.so timeout=10 max-tries=1"))
        self.assertIn(("auth", "include system-auth"), lines)
        self.assertNotIn("pam_deny.so", "".join(rest for _, rest in lines))
        self.assertNotIn("faillock", "".join(rest for _, rest in lines).split("system-auth")[0])

    def test_the_1password_policy_is_installed_with_the_account_named_by_the_installer(self):
        sources = inventory.installer_sources((REPO / "scripts/install-system-config.sh").read_text())
        policy = "/usr/share/polkit-1/actions/com.1password.1Password.policy"
        self.assertEqual(sources["/etc/pam.d/polkit-1"]["source"], "system-config/pam.d/polkit-1")
        self.assertEqual(sources[policy]["source"], "system-config/polkit-1/actions/com.1password.1Password.policy")
        text = (REPO / sources[policy]["source"]).read_text()
        for action in ["unlock", "authorizeCLI", "authorizeSshAgent"]:
            self.assertIn(f'id="com.1password.1Password.{action}"', text)
        self.assertEqual(text.count("unix-user:@USER@"), 2)
        self.assertNotRegex(text, r"unix-user:(?!@USER@)")


class InstallerTable(unittest.TestCase):
    def setUp(self):
        self.sources = inventory.installer_sources((REPO / "scripts/install-system-config.sh").read_text())

    def test_every_installed_source_exists(self):
        for target, entry in self.sources.items():
            self.assertTrue((REPO / entry["source"]).is_file(), f"{target} <- {entry['source']}")

    def test_login_screen_files_are_root_owned_copies(self):
        expected = {
            "/etc/pam.d/dojo-login": "pam.d/dojo-login",
            "/etc/pam.d/dojo-login-fingerprint": "pam.d/dojo-login-fingerprint",
            "/etc/pam.d/dojo-greeter": "pam.d/dojo-greeter",
            "/etc/sysusers.d/dojo-greeter.conf": "sysusers.d/dojo-greeter.conf",
            "/etc/tmpfiles.d/dojo-dm.conf": "tmpfiles.d/dojo-dm.conf",
            "/etc/systemd/system/dojo-dm.service": "systemd/system/dojo-dm.service",
            "/etc/systemd/system/getty@tty2.service.d/dojo-rescue.conf": "systemd/system/getty@tty2.service.d/dojo-rescue.conf",
        }
        for target, source in expected.items():
            self.assertEqual(
                self.sources[target],
                {"source": "system-config/" + source, "installation": "copy", "expected_mode": "0644"},
            )

    def test_ly_is_no_longer_installed(self):
        self.assertNotIn("/etc/pam.d/ly", self.sources)
        self.assertFalse((CONFIG / "pam.d/ly").exists())

    def test_ly_survives_only_as_something_to_disable(self):
        for path in ("scripts/install-system-config.sh", "scripts/enable-keyring-login-unlock.sh",
                     "system-config/systemd/system/update-nix-gl.service", "system-config/packages/official-explicit.txt"):
            self.assertNotRegex((REPO / path).read_text(), r"\bly\b", path)
        for line in (REPO / "scripts/bootstrap-host").read_text().splitlines():
            if re.search(r"\bly\b", line):
                self.assertRegex(line, r"disable|is-enabled", line)

    def test_bootstrap_enables_the_login_screen_and_only_disables_ly(self):
        text = (REPO / "scripts/bootstrap-host").read_text()
        self.assertIn("systemctl enable dojo-dm.service getty@tty2.service", text)
        self.assertNotRegex(text, r"systemctl enable[^\n]*\bly@")
        self.assertIn("disable ly@tty2.service", text)

    def test_keyring_script_checks_dojo_login(self):
        text = (REPO / "scripts/enable-keyring-login-unlock.sh").read_text()
        self.assertIn("/etc/pam.d/dojo-login", text)
        self.assertNotIn("/etc/pam.d/ly", text)


class PamServices(unittest.TestCase):
    def test_every_service_has_an_auth_stack(self):
        for name in ("dojo-login", "dojo-login-fingerprint", "dojo-greeter"):
            self.assertTrue(any(kind == "auth" for kind, _ in stack_lines(name)), name)
            text = (CONFIG / "pam.d" / name).read_text()
            self.assertTrue(any(line.startswith("auth ") for line in text.splitlines()), name)

    def test_password_login_is_the_old_ly_stack_with_a_user_session(self):
        lines = stack_lines("dojo-login")
        self.assertIn(("auth", "include system-login"), lines)
        self.assertIn(("session", "optional pam_gnome_keyring.so auto_start"), lines)
        self.assertIn(("session", "optional pam_systemd.so class=user"), lines)
        self.assertFalse([rest for _, rest in lines if "pam_fprintd" in rest])

    def test_fingerprint_login_is_fingerprint_only_and_bounded(self):
        lines = stack_lines("dojo-login-fingerprint")
        auth = [rest for kind, rest in lines if kind == "auth"]
        self.assertEqual(len(auth), 1)
        self.assertRegex(auth[0], r"^required pam_fprintd\.so timeout=\d+ max-tries=\d+$")
        for _, rest in lines:
            self.assertNotIn("pam_faillock", rest)
            self.assertNotIn("pam_gnome_keyring", rest)
            self.assertNotIn("pam_unix", rest)
        self.assertIn(("session", "optional pam_systemd.so class=user"), lines)

    def test_greeter_session_has_class_greeter_and_no_real_authentication(self):
        lines = stack_lines("dojo-greeter")
        self.assertIn(("auth", "required pam_permit.so"), lines)
        self.assertIn(("session", "optional pam_systemd.so class=greeter"), lines)
        self.assertEqual([rest for kind, rest in lines if kind == "password"], ["required pam_deny.so"])


class Units(unittest.TestCase):
    def test_service_takes_tty1_and_leaves_tty2_when_it_fails(self):
        unit, service, install = (unit_section("dojo-dm.service", s) for s in ("Unit", "Service", "Install"))
        self.assertEqual(unit["Conflicts"], ["getty@tty1.service"])
        after = unit["After"][0].split()
        for required in ("systemd-user-sessions.service", "plymouth-quit-wait.service", "systemd-logind.service"):
            self.assertIn(required, after)
        self.assertEqual((unit["StartLimitBurst"], unit["StartLimitIntervalSec"]), (["3"], ["30"]))
        self.assertEqual(install["Alias"], ["display-manager.service"])
        self.assertEqual(service["Restart"], ["always"])
        self.assertEqual((service["TTYPath"], service["StandardInput"]), (["/dev/tty1"], ["tty"]))
        self.assertEqual((service["RuntimeDirectory"], service["StateDirectory"]), (["dojo-dm"], ["dojo-dm"]))

    def test_service_sets_nothing_the_desktop_sessions_would_inherit(self):
        service = unit_section("dojo-dm.service", "Service")
        self.assertEqual(service["KeyringMode"], ["private"])
        inherited = ("NoNewPrivileges", "ProtectSystem", "ProtectHome", "PrivateTmp", "PrivateDevices",
                     "PrivateMounts", "PrivateNetwork", "CapabilityBoundingSet", "AmbientCapabilities",
                     "SystemCallFilter", "SystemCallArchitectures", "MemoryDenyWriteExecute",
                     "RestrictSUIDSGID", "RestrictNamespaces", "RestrictAddressFamilies", "LockPersonality",
                     "ProtectKernelTunables", "ProtectKernelModules", "ProtectProc", "ProtectClock",
                     "IPAddressDeny", "UMask", "OOMScoreAdjust", "LimitCORE", "WatchdogSec")
        for key in inherited:
            self.assertNotIn(key, service, f"{key} would reach every session this service starts")

    def test_service_runs_the_installed_release_without_devenv(self):
        text = (CONFIG / "systemd/system/dojo-dm.service").read_text()
        self.assertEqual(unit_section("dojo-dm.service", "Service")["ExecStart"], ["/usr/local/lib/dojo-dm/current/run"])
        self.assertNotIn("devenv", text)
        self.assertNotIn("/home", text)

    def test_nix_gl_is_ordered_before_the_display_manager(self):
        before = unit_section("update-nix-gl.service", "Unit")["Before"][0].split()
        self.assertEqual(before, ["display-manager.service", "dojo-dm.service"])

    def test_greeter_user_has_no_home_no_shell_and_no_groups(self):
        entries = [shlex.split(line) for line in (CONFIG / "sysusers.d/dojo-greeter.conf").read_text().splitlines()
                   if line and not line.startswith("#")]
        self.assertEqual(len(entries), 1)
        kind, name, uid, gecos, home, shell = entries[0]
        self.assertEqual((kind, name, uid, home, shell), ("u", "dojo-greeter", "-", "-", "/usr/bin/nologin"))

    def test_snapshots_are_readable_only_by_root_and_the_greeter(self):
        lines = [line.split() for line in (CONFIG / "tmpfiles.d/dojo-dm.conf").read_text().splitlines()
                 if line and not line.startswith("#")]
        self.assertEqual(lines, [["d", "/var/lib/dojo-dm/users", "0750", "root", "dojo-greeter", "-"]])

    def test_greeter_niri_has_one_client_and_no_key_bindings(self):
        text = (CONFIG / "dojo-dm/greeter.kdl").read_text()
        self.assertEqual(len(re.findall(r"^spawn(-sh)?-at-startup", text, re.M)), 1)
        self.assertNotRegex(text, r"^\s*binds\b")
        self.assertNotRegex(text, r"^\s*include\b")


if __name__ == "__main__":
    unittest.main()
