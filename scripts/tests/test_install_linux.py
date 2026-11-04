#!/usr/bin/env python3
"""Isolated installer checks: fake Cargo output, temporary prefix and XDG paths.

Run with python3 scripts/tests/test_install_linux.py. No real build or user
installation is performed. gio/desktop-file-validate checks run when available.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest


REPO = Path(__file__).resolve().parents[2]


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux installer")
class LinuxInstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="entocad-installer-")
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.repo = self.base / 'checkout with spaces "quoted" \\ $cash %literal `tick` ö'
        self.prefix = self.base / 'install with spaces "quoted" \\ $cash %literal `tick` ö'
        self.data = self.base / "menu data"
        self.state = self.base / "state"
        self.repo.mkdir()
        (self.repo / "scripts").mkdir()
        (self.repo / "packaging/linux").mkdir(parents=True)
        (self.repo / "resources/icons").mkdir(parents=True)
        shutil.copy2(REPO / "scripts/install-linux.sh", self.repo / "scripts/install-linux.sh")
        shutil.copy2(REPO / "packaging/linux/install.sh", self.repo / "packaging/linux/install.sh")
        shutil.copy2(REPO / "resources/icons/entocad-256.png", self.repo / "resources/icons/entocad-256.png")
        (self.repo / "Cargo.lock").write_text("# Fixture lockfile\n")
        self.set_version("0.60.0")
        self.fake_tools = self.base / "fake tools"
        self.fake_tools.mkdir()
        self.fixture = self.base / "fixture app"
        self.set_app("old")
        self.write_tool("cargo", r'''#!/usr/bin/env python3
import json, os, pathlib, shutil, sys, time
pathlib.Path(os.environ["TEST_CARGO_ARGS"]).write_text(json.dumps(sys.argv[1:]))
time.sleep(float(os.environ.get("TEST_BUILD_DELAY", "0")))
if os.environ.get("TEST_BUILD_FAIL") == "1":
    print("deliberate fixture build failure", file=sys.stderr)
    sys.exit(23)
output = pathlib.Path(os.environ.get("TEST_ARTIFACT_DIR", str(pathlib.Path.cwd() / "custom target/native-triple/release"))) / "mycad"
output.parent.mkdir(parents=True, exist_ok=True)
shutil.copy2(os.environ["TEST_APP_FIXTURE"], output)
print(json.dumps({"reason": "compiler-artifact", "target": {"name": "mycad", "kind": ["lib"]}, "executable": "/wrong/artifact"}))
print(json.dumps({"reason": "compiler-artifact", "target": {"name": "mycad", "kind": ["bin"]}, "executable": str(output)}))
''')
        # Cache tools stay entirely within the test fixture.
        self.write_tool("update-desktop-database", "#!/bin/sh\nexit 0\n")
        self.write_tool("gtk-update-icon-cache", "#!/bin/sh\nexit 0\n")
        self.write_tool("notify-send", '#!/bin/sh\nprintf "%s\\n" "$@" > "$TEST_NOTIFICATION"\n')
        self.env = dict(os.environ)
        self.env.update({
            "PATH": str(self.fake_tools) + os.pathsep + os.environ["PATH"],
            "XDG_DATA_HOME": str(self.data),
            "XDG_STATE_HOME": str(self.state),
            "TEST_CARGO_ARGS": str(self.base / "cargo-args.json"),
            "TEST_APP_FIXTURE": str(self.fixture),
            "TEST_LAUNCH_OUTPUT": str(self.base / "launched.json"),
            "TEST_NOTIFICATION": str(self.base / "notification.txt"),
        })
        for name in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET", "ENTOCAD_UPDATE_LOG_CAPTURED"):
            self.env.pop(name, None)
        self.bin = self.prefix / "bin/entocad"
        self.manifest = self.prefix / "bin/entocad.source.json"
        self.desktop = self.data / "applications/entocad.desktop"
        self.icon = self.data / "icons/hicolor/256x256/apps/entocad.png"
        self.log = self.state / "entocad/install.log"

    def write_tool(self, name, contents):
        path = self.fake_tools / name
        path.write_text(contents)
        path.chmod(0o755)

    def set_version(self, version):
        (self.repo / "Cargo.toml").write_text(f'[workspace]\nmembers = []\n[workspace.package]\nversion = "{version}"\nedition = "2021"\n')

    def set_app(self, version):
        self.fixture.write_text(f'''#!/usr/bin/env python3
import json, os, pathlib, sys, time
pathlib.Path(os.environ["TEST_LAUNCH_OUTPUT"]).write_text(json.dumps({{"version": {version!r}, "args": sys.argv[1:]}}))
time.sleep(float(os.environ.get("TEST_APP_SLEEP", "0")))
''')
        self.fixture.chmod(0o755)

    def command(self, *args):
        return ["bash", str(self.repo / "scripts/install-linux.sh"), "--prefix", str(self.prefix), *args]

    def install(self, *args, expect=0, env=None):
        result = subprocess.run(self.command(*args), env=env or self.env, text=True, capture_output=True, timeout=20)
        if expect == 0:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def launched(self):
        path = Path(self.env["TEST_LAUNCH_OUTPUT"])
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            try:
                return json.loads(path.read_text())
            except (FileNotFoundError, json.JSONDecodeError):
                time.sleep(0.02)
        self.fail("The installed application was not launched")

    def artifacts(self):
        return {path: path.read_bytes() for path in (self.bin, self.manifest, self.desktop, self.icon)}

    def test_install_and_update_keep_launcher_and_record_source(self):
        self.install()
        info = json.loads(self.manifest.read_text())
        self.assertEqual(info["protocol_version"], 1)
        self.assertEqual(info["repo"], str(self.repo))
        self.assertEqual(info["update_command"], str(self.repo / "scripts/install-linux.sh"))
        self.assertEqual(info["installed_version"], "0.60.0")
        self.assertEqual(info["install_prefix"], str(self.prefix))
        self.assertEqual(info["data_dir"], str(self.data))
        self.assertTrue(os.access(self.bin, os.X_OK))
        self.assertFalse(Path(self.env["TEST_LAUNCH_OUTPUT"]).exists())
        args = json.loads(Path(self.env["TEST_CARGO_ARGS"]).read_text())
        self.assertEqual(args[:5], ["build", "--release", "-p", "mycad", "--locked"])
        desktop_before = self.desktop.read_bytes()
        self.set_version("0.61.0")
        self.set_app("new")
        self.install()
        self.assertEqual(self.desktop.read_bytes(), desktop_before)
        self.assertEqual(json.loads(self.manifest.read_text())["installed_version"], "0.61.0")
        subprocess.run([str(self.bin)], env=self.env, check=True, timeout=5)
        self.assertEqual(self.launched()["version"], "new")
        self.assertIn("EntoCAD 0.61.0 is installed", self.log.read_text())
        self.assertEqual(list(self.prefix.rglob(".entocad-install.*[0-9A-Za-z]")), [self.prefix / "bin/.entocad-install.lock"])

    def test_build_failure_preserves_working_install_and_does_not_launch(self):
        self.install()
        original = self.artifacts()
        self.set_version("0.61.0")
        self.set_app("new")
        env = dict(self.env, TEST_BUILD_FAIL="1")
        result = self.install("--launch", expect=1, env=env)
        self.assertEqual(self.artifacts(), original)
        self.assertFalse(Path(self.env["TEST_LAUNCH_OUTPUT"]).exists())
        self.assertIn("installed app was not changed", result.stdout)
        self.assertIn("deliberate fixture build failure", self.log.read_text())
        self.assertIn(str(self.log), Path(self.env["TEST_NOTIFICATION"]).read_text())

    def test_partial_replace_failure_rolls_back_all_artifacts(self):
        self.install()
        original = self.artifacts()
        self.set_version("0.61.0")
        self.set_app("new")
        self.write_tool("mv", '''#!/usr/bin/env python3
import os, pathlib, sys
args = sys.argv[1:]
if pathlib.Path(args[-2]).name == "new" and args[-1].endswith(".desktop"):
    print("deliberate desktop replacement failure", file=sys.stderr)
    sys.exit(24)
os.execv("/usr/bin/mv", ["mv", *args])
''')
        self.install("--launch", expect=1)
        self.assertEqual(self.artifacts(), original)
        self.assertFalse(Path(self.env["TEST_LAUNCH_OUTPUT"]).exists())

    def test_missing_cargo_artifact_preserves_working_install(self):
        self.install()
        original = self.artifacts()
        self.write_tool("cargo", '#!/bin/sh\nprintf \'%s\\n\' \'{"reason":"build-finished","success":true}\'\n')
        self.install(expect=1)
        self.assertEqual(self.artifacts(), original)
        self.assertIn("Cargo did not report the mycad executable", self.log.read_text())

    def test_menu_environment_finds_rust_in_cargo_home(self):
        # Desktop sessions need not include the user's Rust directory in PATH.
        # Supply only the system tools the installer needs, and fake Cargo in
        # its standard CARGO_HOME/bin layout; never change the actual HOME.
        desktop_path = self.base / "desktop PATH"
        desktop_path.mkdir()
        for name in ("bash", "uname", "readlink", "dirname", "mkdir", "touch", "tee", "date", "flock", "awk", "git", "mktemp", "python3", "install", "chmod", "cp", "mv", "rm"):
            (desktop_path / name).symlink_to(shutil.which(name))
        cargo_home = self.base / "rust home"
        (cargo_home / "bin").mkdir(parents=True)
        (cargo_home / "bin/cargo").symlink_to(self.fake_tools / "cargo")
        env = dict(self.env, PATH=str(desktop_path), CARGO_HOME=str(cargo_home))
        self.install(env=env)
        self.assertTrue(self.bin.is_file())
        self.assertTrue(Path(self.env["TEST_CARGO_ARGS"]).is_file())

    def test_launch_reopens_exact_argument_and_uses_explicit_data_dir(self):
        drawing = self.base / 'drawing with spaces "$x" %f ; `literal`.dwg'
        drawing.write_text("fixture")
        custom_data = self.base / "custom data"
        self.install("--data-dir", str(custom_data), "--launch", "--reopen", str(drawing))
        self.assertEqual(self.launched()["args"], [str(drawing)])
        self.assertTrue((custom_data / "applications/entocad.desktop").is_file())
        self.assertFalse(self.desktop.exists())
        self.assertEqual(json.loads(self.manifest.read_text())["data_dir"], str(custom_data))

    def test_desktop_launcher_handles_special_path_characters(self):
        self.install()
        validator = shutil.which("desktop-file-validate")
        if validator:
            result = subprocess.run([validator, str(self.desktop)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        gio = shutil.which("gio")
        if not gio:
            self.skipTest("gio unavailable; install behavior covered separately")
        drawing = self.base / "library drawing.dwg"
        drawing.touch()
        result = subprocess.run([gio, "launch", str(self.desktop), str(drawing)], env=self.env, capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.launched()["args"], [str(drawing)])

    def test_waits_for_old_process_without_terminating_it(self):
        old = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(0.6)"])
        self.addCleanup(lambda: old.wait(timeout=5))
        start = time.monotonic()
        self.install("--wait-pid", str(old.pid), "--launch")
        self.assertGreaterEqual(time.monotonic() - start, 0.5)
        self.assertEqual(old.wait(timeout=1), 0)
        self.assertEqual(self.launched()["version"], "old")

    def test_concurrent_install_is_rejected(self):
        env = dict(self.env, TEST_BUILD_DELAY="0.8")
        first = subprocess.Popen(self.command(), env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            deadline = time.monotonic() + 5
            while not Path(self.env["TEST_CARGO_ARGS"]).exists() and time.monotonic() < deadline:
                time.sleep(0.02)
            self.assertFalse(first.poll() is not None, "first installer stopped early")
            result = self.install(expect=1)
            self.assertIn("Another EntoCAD installer is running", result.stdout)
        finally:
            output, error = first.communicate(timeout=10)
        self.assertEqual(first.returncode, 0, output + error)

    def test_uninstall_keeps_user_documents_and_other_apps(self):
        self.install()
        drawing = self.prefix / "my drawing.dwg"
        other_app = self.prefix / "bin/another-app"
        drawing.write_text("saved drawing")
        other_app.write_text("another app")
        self.install("--uninstall")
        for path in (self.bin, self.manifest, self.desktop, self.icon):
            self.assertFalse(path.exists(), path)
        self.assertEqual(drawing.read_text(), "saved drawing")
        self.assertEqual(other_app.read_text(), "another app")

    def test_validation_fails_before_installing(self):
        for args in (("--wait-pid", "0"), ("--wait-pid", "x"), ("--prefix", "relative"), ("--reopen", "/tmp/drawing"), ("--launch", "--reopen", "relative"), ("--uninstall", "--launch"), ("--data-dir", "relative"), ("--log-file", "relative")):
            with self.subTest(args=args):
                self.install(*args, expect=1)
                self.assertFalse(self.bin.exists())

    def test_legacy_wrapper_adds_launch_only_for_old_argument_free_updates(self):
        self.write_tool("capture-args", '''#!/usr/bin/env python3
import json, os, pathlib, sys
pathlib.Path(os.environ["TEST_LAUNCH_OUTPUT"]).write_text(json.dumps(sys.argv[1:]))
''')
        (self.repo / "scripts/install-linux.sh").write_text('#!/usr/bin/env bash\nexec capture-args "$@"\n')
        wrapper = ["bash", str(self.repo / "packaging/linux/install.sh")]
        subprocess.run(wrapper, env=self.env, check=True, timeout=5)
        self.assertEqual(json.loads(Path(self.env["TEST_LAUNCH_OUTPUT"]).read_text()), ["--launch"])
        subprocess.run([*wrapper, "--uninstall", "--prefix", str(self.prefix)], env=self.env, check=True, timeout=5)
        self.assertEqual(json.loads(Path(self.env["TEST_LAUNCH_OUTPUT"]).read_text()), ["--uninstall", "--prefix", str(self.prefix)])


if __name__ == "__main__":
    unittest.main(verbosity=2)
