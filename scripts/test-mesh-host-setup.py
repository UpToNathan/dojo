#!/usr/bin/env python3
"""Run the mesh host-setup scripts against a private directory and fake sshd, systemctl, ssh and PAM."""

import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
HARDENING = REPO / "system-config/ssh/sshd_config.d/10-dojo-mesh.conf"


def script(path, body):
    path.write_text(f"#!/bin/sh\n{body}\n")
    path.chmod(0o755)


class Host(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.ssh_dir = self.root / "etc-ssh"
        self.ssh_dir.mkdir()
        (self.ssh_dir / "sshd_config").write_text("Include /etc/ssh/sshd_config.d/*.conf\nUsePAM yes\n")
        self.home = self.root / "home"
        (self.home / ".ssh").mkdir(parents=True)
        self.log = self.root / "calls"
        script(self.bin / "sshd", f'echo "sshd $@" >> {self.log}; exit "${{FAKE_SSHD_STATUS:-0}}"')
        script(self.bin / "systemctl", f'echo "systemctl $@" >> {self.log}; if [ "$1" = is-active ]; then [ "$3" = sshd.service ]; fi')

    def run_script(self, name, *args, env=None, tool_path=True):
        environment = {
            "PATH": f"{self.bin}:{os.environ['PATH']}",
            "DOJO_NO_SUDO": "1",
            "DOJO_SSH_DIR": str(self.ssh_dir),
            "DOJO_SSHD": str(self.bin / "sshd"),
            "DOJO_SYSTEMCTL": str(self.bin / "systemctl"),
            "DOJO_TARGET_HOME": str(self.home),
            "HOME": str(self.home),
            **(env or {}),
        }
        return subprocess.run([str(REPO / "scripts" / name), *args], env=environment, capture_output=True, text=True)

    def calls(self):
        return self.log.read_text() if self.log.exists() else ""


class InstallMeshSshd(Host):
    dropin = property(lambda self: self.ssh_dir / "sshd_config.d/10-dojo-mesh.conf")
    listen = property(lambda self: self.ssh_dir / "sshd_config.d/11-dojo-mesh-listen.conf")

    def authorize(self):
        (self.home / ".ssh/authorized_keys").write_text("# mine\nssh-ed25519 AAAAC3 me\n")

    def test_the_tracked_hardening_turns_passwords_and_root_off(self):
        text = HARDENING.read_text()
        for setting in ["PasswordAuthentication no", "KbdInteractiveAuthentication no", "PermitRootLogin no", "AuthenticationMethods publickey"]:
            self.assertIn(setting + "\n", text)

    def test_a_user_with_no_key_is_not_locked_out_of_their_own_machine(self):
        result = self.run_script("install-mesh-sshd.sh")
        self.assertEqual(result.returncode, 1)
        self.assertIn("admit nobody", result.stderr)
        self.assertFalse(self.dropin.exists())
        self.assertEqual(self.calls(), "")

    def test_it_installs_a_copy_checks_it_and_reloads_without_restarting(self):
        self.authorize()
        result = self.run_script("install-mesh-sshd.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.dropin.read_text(), HARDENING.read_text())
        self.assertFalse(self.dropin.is_symlink())
        self.assertEqual(stat.S_IMODE(self.dropin.stat().st_mode), 0o644)
        calls = self.calls().splitlines()
        self.assertEqual(calls[0], f"sshd -t -f {self.ssh_dir}/sshd_config")
        self.assertIn("systemctl reload sshd.service", calls)
        self.assertFalse(any("restart" in call for call in calls))

    def test_running_it_again_changes_nothing(self):
        self.authorize()
        self.run_script("install-mesh-sshd.sh")
        again = self.run_script("install-mesh-sshd.sh")
        self.assertEqual(again.returncode, 0)
        self.assertNotIn("Installed", again.stdout)
        self.assertEqual([p.name for p in (self.ssh_dir / "sshd_config.d").iterdir()], ["10-dojo-mesh.conf"])

    def test_a_file_it_replaces_is_kept_as_a_backup(self):
        self.authorize()
        self.dropin.parent.mkdir()
        self.dropin.write_text("PasswordAuthentication yes\n")
        result = self.run_script("install-mesh-sshd.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        backups = list(self.dropin.parent.glob("10-dojo-mesh.conf.backup-*"))
        self.assertEqual(len(backups), 1)
        self.assertEqual(backups[0].read_text(), "PasswordAuthentication yes\n")

    def test_a_configuration_sshd_rejects_is_undone(self):
        self.authorize()
        self.dropin.parent.mkdir()
        self.dropin.write_text("# the previous drop-in\n")
        result = self.run_script("install-mesh-sshd.sh", env={"FAKE_SSHD_STATUS": "1"})
        self.assertEqual(result.returncode, 1)
        self.assertIn("putting the old files back", result.stderr)
        self.assertEqual(self.dropin.read_text(), "# the previous drop-in\n")
        self.assertNotIn("reload", self.calls())

    def test_listening_only_where_asked_and_going_back_when_not(self):
        self.authorize()
        result = self.run_script("install-mesh-sshd.sh", "--listen", "192.0.2.10", "--listen", "fd00::1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("ListenAddress 192.0.2.10\nListenAddress fd00::1\n", self.listen.read_text())
        again = self.run_script("install-mesh-sshd.sh")
        self.assertEqual(again.returncode, 0, again.stderr)
        self.assertFalse(self.listen.exists())
        self.assertEqual(len(list(self.listen.parent.glob("11-dojo-mesh-listen.conf.backup-*"))), 1)

    def test_a_port_other_than_22_can_be_chosen_for_where_something_else_owns_it(self):
        self.authorize()
        result = self.run_script("install-mesh-sshd.sh", "--port", "2222")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Port 2222\n", self.listen.read_text())
        self.assertNotIn("ListenAddress", self.listen.read_text())
        for bad in ["0", "65536", "ssh", "22;"]:
            self.assertEqual(self.run_script("install-mesh-sshd.sh", "--port", bad).returncode, 2, bad)

    def test_a_listen_value_that_is_not_an_address_is_refused(self):
        self.authorize()
        for bad in ["a b", "x;y", "$(id)", ""]:
            self.assertEqual(self.run_script("install-mesh-sshd.sh", "--listen", bad).returncode, 2, bad)
        self.assertFalse(self.dropin.exists())

    def test_allow_no_keys_is_the_way_to_harden_a_machine_reached_another_way(self):
        result = self.run_script("install-mesh-sshd.sh", "--allow-no-keys")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.dropin.exists())

    def test_it_says_so_when_the_main_configuration_does_not_read_drop_ins(self):
        self.authorize()
        (self.ssh_dir / "sshd_config").write_text("UsePAM yes\n")
        result = self.run_script("install-mesh-sshd.sh")
        self.assertIn("does not include sshd_config.d", result.stderr)


class CheckMeshSshd(Host):
    def probe(self, reply):
        script(self.bin / "ssh", f"echo '{reply}' >&2; exit 255")
        return self.run_script("check-mesh-sshd.sh", "peer.example")

    def test_only_public_keys_offered_passes(self):
        result = self.probe("dojo-probe@peer.example: Permission denied (publickey).")
        self.assertEqual((result.returncode, result.stdout.strip()), (0, "peer.example accepts only: publickey"))

    def test_a_server_still_offering_passwords_fails(self):
        for methods in ["publickey,password", "publickey,keyboard-interactive", "password"]:
            result = self.probe(f"dojo-probe@peer.example: Permission denied ({methods}).")
            self.assertEqual(result.returncode, 1, methods)
            self.assertIn(methods, result.stderr)

    def test_an_unreachable_server_is_not_a_pass(self):
        result = self.probe("ssh: connect to host peer.example port 22: Connection refused")
        self.assertEqual(result.returncode, 2)
        self.assertIn("Connection refused", result.stderr)

    def test_the_probe_never_offers_a_key_or_prompts(self):
        script(self.bin / "ssh", f'echo "$@" > {self.root}/args; echo "x: Permission denied (publickey)." >&2; exit 255')
        self.run_script("check-mesh-sshd.sh", "peer.example", "2222")
        args = (self.root / "args").read_text()
        for needed in ["BatchMode=yes", "PubkeyAuthentication=no", "-F /dev/null", "-p 2222"]:
            self.assertIn(needed, args)


class SudoYubikey(Host):
    def setUp(self):
        super().setUp()
        self.pam = self.root / "pam.d"
        self.pam.mkdir()
        self.sudo = self.pam / "sudo"
        self.original = "#%PAM-1.0\nauth      include     system-auth\naccount   include     system-auth\n"
        self.sudo.write_text(self.original)
        self.module = self.root / "pam_u2f.so"
        self.mapping = self.root / "u2f-mappings"
        self.env = {"DOJO_PAM_DIR": str(self.pam), "DOJO_PAM_U2F": str(self.module), "DOJO_U2F_MAPPING": str(self.mapping)}

    def ready(self):
        self.module.write_text("")
        self.mapping.write_text("pat:keyhandle,publickey,es256,+presence\n")

    def test_it_will_not_enable_without_the_module_or_a_registered_key(self):
        self.mapping.write_text("pat:x\n")
        self.assertEqual(self.run_script("enable-sudo-yubikey.sh", "enable", env=self.env).returncode, 1)
        self.module.write_text("")
        self.mapping.write_text("")
        result = self.run_script("enable-sudo-yubikey.sh", "enable", env=self.env)
        self.assertEqual(result.returncode, 1)
        self.assertIn("register", result.stderr)
        self.assertEqual(self.sudo.read_text(), self.original)

    def test_enabling_puts_one_line_before_the_first_auth_and_keeps_a_backup(self):
        self.ready()
        result = self.run_script("enable-sudo-yubikey.sh", "enable", env=self.env)
        self.assertEqual(result.returncode, 0, result.stderr)
        lines = self.sudo.read_text().splitlines()
        self.assertEqual(lines[0], "#%PAM-1.0")
        self.assertTrue(lines[1].startswith("# dojo-mesh"))
        self.assertIn("pam_u2f.so authfile=" + str(self.mapping), lines[2])
        self.assertIn("nouserok", lines[2])
        self.assertEqual(lines[3], "auth      include     system-auth")
        self.assertEqual(len(list(self.pam.glob("sudo.backup-*"))), 1)
        again = self.run_script("enable-sudo-yubikey.sh", "enable", env=self.env)
        self.assertIn("Already enabled", again.stdout)
        self.assertEqual(self.sudo.read_text().count("pam_u2f"), 1)

    def test_disabling_restores_what_was_there(self):
        self.ready()
        self.run_script("enable-sudo-yubikey.sh", "enable", env=self.env)
        result = self.run_script("enable-sudo-yubikey.sh", "disable", env=self.env)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.sudo.read_text(), self.original)
        self.assertIn("Not enabled", self.run_script("enable-sudo-yubikey.sh", "disable", env=self.env).stdout)

    def test_registering_appends_to_a_root_owned_mapping_and_keeps_earlier_keys(self):
        script(self.bin / "pamu2fcfg", 'echo "$2:handle2,pub2,es256,+presence"')
        self.mapping.write_text("pat:handle1,pub1,es256,+presence\n")
        result = self.run_script("enable-sudo-yubikey.sh", "register", "--user", "pat", env=self.env)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.mapping.read_text(), "pat:handle1,pub1,es256,+presence\npat:handle2,pub2,es256,+presence\n")

    def test_a_key_that_was_not_touched_registers_nothing(self):
        script(self.bin / "pamu2fcfg", "exit 1")
        result = self.run_script("enable-sudo-yubikey.sh", "register", env=self.env)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.mapping.exists())


class OnboardMesh(Host):
    def setUp(self):
        super().setUp()
        self.mesh_calls = self.root / "mesh-calls"
        script(self.bin / "dojo-mesh", f'echo "$@" >> {self.mesh_calls}')
        script(self.bin / "ssh-add", "echo ssh-ed25519 AAAA key")
        script(self.bin / "op", "true")
        script(self.bin / "tailscale", "echo 100.71.60.101")
        (self.ssh_dir / "ssh_host_ed25519_key.pub").write_text("ssh-ed25519 AAAAHOST host\n")
        self.env = {"DOJO_MESH": str(self.bin / "dojo-mesh"), "DOJO_HOST_KEY": str(self.ssh_dir / "ssh_host_ed25519_key.pub")}

    def onboard(self, *args, env=None):
        return self.run_script("onboard-mesh.sh", *args, env={**self.env, **(env or {})})

    def test_it_sets_sshd_up_on_its_own_port_and_onboards_with_the_primary_key_first(self):
        script(self.bin / "systemctl", f'echo "systemctl $@" >> {self.log}; [ "$1" != is-active ]')
        result = self.onboard("--name", "orchard", "--sign", "yubikey", "--sign", "1password")
        self.assertEqual(result.returncode, 0, result.stderr)
        listen = (self.ssh_dir / "sshd_config.d/11-dojo-mesh-listen.conf").read_text()
        self.assertIn("Port 2222\n", listen)
        self.assertIn("systemctl enable --now sshd.service", self.calls())
        call = self.mesh_calls.read_text().strip()
        self.assertEqual(
            call,
            f"onboard --node-name orchard --route 100.71.60.101:2222 --sign yubikey --sign 1password --host-key-file {self.ssh_dir}/ssh_host_ed25519_key.pub",
        )

    def test_a_machine_that_signs_nothing_still_onboards(self):
        result = self.onboard("--name", "guest", "--route", "192.0.2.7", "--port", "2200")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--route 192.0.2.7:2200", self.mesh_calls.read_text())
        self.assertNotIn("--sign", self.mesh_calls.read_text())
        self.assertIn("not a tailnet address", result.stdout)

    def test_it_stops_before_changing_anything_when_1password_is_not_ready(self):
        script(self.bin / "ssh-add", "echo 'Error connecting to agent' >&2; exit 2")
        result = self.onboard("--name", "orchard", "--sign", "yubikey")
        self.assertEqual(result.returncode, 3)
        self.assertIn("Unlock 1Password", result.stderr)
        self.assertFalse((self.ssh_dir / "sshd_config.d").exists())
        self.assertFalse(self.mesh_calls.exists())

    def test_sshd_already_running_is_left_running(self):
        script(self.bin / "systemctl", f'echo "systemctl $@" >> {self.log}; if [ "$1" = is-active ]; then exit 0; fi')
        result = self.onboard("--name", "orchard", "--sign", "yubikey")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("enable --now", self.calls())

    def test_no_host_leaves_sshd_alone(self):
        result = self.onboard("--name", "orchard", "--no-host", "--sign", "yubikey")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls(), "")
        self.assertFalse((self.ssh_dir / "sshd_config.d").exists())

    def test_a_port_that_is_not_a_port_is_refused(self):
        self.assertEqual(self.onboard("--port", "70000").returncode, 2)


class InstallOnePasswordCli(Host):
    def setUp(self):
        super().setUp()
        self.source = self.root / "op-source"
        script(self.source, 'echo "2.99.0"')
        self.target = self.root / "usr-local-bin-op"

    def install(self, *args):
        return self.run_script("install-onepassword-cli.sh", *args, env={"DOJO_OP_TARGET": str(self.target)})

    def test_it_installs_the_cli_with_the_setgid_bit_the_desktop_integration_needs(self):
        result = self.install("--from", str(self.source))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(stat.S_IMODE(self.target.stat().st_mode), 0o2755)
        self.assertIn("2.99.0", result.stdout)

    def test_running_it_again_changes_nothing_and_a_different_one_is_kept_as_a_backup(self):
        self.install("--from", str(self.source))
        again = self.install("--from", str(self.source))
        self.assertIn("already installed", again.stdout)
        script(self.source, 'echo "3.0.0"')
        newer = self.install("--from", str(self.source))
        self.assertEqual(newer.returncode, 0, newer.stderr)
        self.assertEqual(len(list(self.root.glob("usr-local-bin-op.backup-*"))), 1)
        self.assertIn("3.0.0", newer.stdout)

    def test_without_a_binary_to_install_it_says_so(self):
        result = self.run_script("install-onepassword-cli.sh", env={"DOJO_OP_TARGET": str(self.target), "HOME": str(self.root / "nowhere")})
        self.assertEqual(result.returncode, 1)
        self.assertIn("--from", result.stderr)
        self.assertFalse(self.target.exists())


if __name__ == "__main__":
    unittest.main()
