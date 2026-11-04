#!/usr/bin/env python3
"""Exercise macOS installation transactions with native tools mocked on Linux.

Run: python3 -m unittest discover -s scripts/tests -p test_install_macos.py
These tests do not validate macOS signing, LaunchServices, or native tool output.
"""

import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile
import threading
import unittest


REPOSITORY = Path(__file__).resolve().parents[2]
MOCK_TOOL = r'''#!/usr/bin/env python3
import json, os, pathlib, plistlib, subprocess, sys
tool = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ["MOCK_CALLS"], "a") as out:
    out.write(json.dumps([tool] + args) + "\n")
if tool == "uname":
    print(os.environ.get("MOCK_OS", "Darwin"))
elif tool == "cargo":
    if os.environ.get("MOCK_BUILD_FAIL"):
        print("simulated Cargo build failure", file=sys.stderr)
        sys.exit(1)
    binary = pathlib.Path(os.environ["MOCK_ARTIFACT"])
    binary.parent.mkdir(parents=True, exist_ok=True)
    binary.write_text(os.environ.get("MOCK_BINARY", "first build"))
    binary.chmod(0o755)
    print(json.dumps({"reason": "compiler-artifact", "target": {"name": "mycad", "kind": ["bin"]}, "executable": str(binary)}))
    print(json.dumps({"reason": "build-finished", "success": True}))
elif tool == "plutil":
    source = pathlib.Path(args[-1])
    try:
        data = source.read_bytes()
        obj = json.loads(data) if data.lstrip().startswith(b"{") else plistlib.loads(data)
    except (ValueError, OSError):
        sys.exit(1)
    if args[0] == "-extract":
        try:
            for key in args[1].split("."):
                obj = obj[int(key)] if isinstance(obj, list) else obj[key]
            print(obj)
        except (KeyError, IndexError):
            sys.exit(1)
    elif args[0] in ("-replace", "-insert"):
        obj[args[1]] = int(args[3]) if args[2] == "-integer" else args[3]
        source.write_bytes(plistlib.dumps(obj))
    elif args[0] == "-convert":
        pathlib.Path(args[args.index("-o") + 1]).write_text(json.dumps(obj))
    elif args[0] != "-lint":
        raise ValueError(args)
elif tool == "sips":
    pathlib.Path(args[args.index("--out") + 1]).write_bytes(b"mock png")
elif tool == "iconutil":
    if os.environ.get("MOCK_ICON_FAIL"):
        print("simulated icon generation failure", file=sys.stderr)
        sys.exit(1)
    pathlib.Path(args[args.index("-o") + 1]).write_bytes(b"mock icns")
elif tool == "mv":
    if os.environ.get("MOCK_PUBLISH_FAIL") and pathlib.Path(args[0]).name == "EntoCAD.app" and ".entocad-install." in args[0]:
        print("simulated app publication failure", file=sys.stderr)
        sys.exit(1)
    sys.exit(subprocess.call(["/bin/mv"] + args))
elif tool == "osascript":
    print("Close")
elif tool != "open":
    raise ValueError(tool)
'''


class MacInstallerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="entocad mac tests ")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.repo = self.root / 'checkout "with spaces"'
        self.applications = self.root / 'Applications "personal"'
        self.app = self.applications / "EntoCAD.app"
        self.binary = self.app / "Contents/MacOS/EntoCAD"
        self.manifest = self.app / "Contents/MacOS/EntoCAD.source.json"
        self.log = self.root / "logs/install.log"
        self.calls = self.root / "calls.jsonl"
        for relative in (
            "scripts/install-macos.sh",
            "packaging/macos/Info.plist",
            "resources/icons/entocad-256.png",
        ):
            target = self.repo / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(REPOSITORY / relative, target)
        (self.repo / "Cargo.toml").write_text('[workspace.package]\nversion = "1.2.3"\n')
        mock_dir = self.root / "tools"
        mock_dir.mkdir()
        for tool in ("uname", "cargo", "plutil", "sips", "iconutil", "open", "mv", "osascript"):
            script = mock_dir / tool
            script.write_text(MOCK_TOOL)
            script.chmod(0o755)
        self.env = dict(os.environ)
        self.env.update(
            PATH=str(mock_dir) + os.pathsep + os.environ["PATH"],
            MOCK_CALLS=str(self.calls),
            MOCK_ARTIFACT=str(self.root / "configured cargo output/release/mycad"),
        )

    def run_installer(self, *args, expected=0, **environment):
        result = subprocess.run(
            [
                "bash",
                str(self.repo / "scripts/install-macos.sh"),
                "--install-dir",
                str(self.applications),
                "--log-file",
                str(self.log),
                *args,
            ],
            env={**self.env, **environment},
            capture_output=True,
            text=True,
            timeout=30,
        )
        self.assertEqual(result.returncode, expected, result.stdout + result.stderr)
        return result

    def tool_calls(self, tool):
        if not self.calls.exists():
            return []
        return [call for line in self.calls.read_text().splitlines() if (call := json.loads(line))[0] == tool]

    def assert_clean_transaction(self):
        self.assertFalse(list(self.applications.glob(".entocad-install.*")))

    def test_install_and_update_use_stable_bundle_and_custom_cargo_output(self):
        self.run_installer()
        self.assertEqual(self.binary.read_text(), "first build")
        manifest = json.loads(self.manifest.read_text())
        self.assertEqual(manifest["repo"], str(self.repo))
        self.assertEqual(manifest["update_command"], str(self.repo / "scripts/install-macos.sh"))
        self.assertEqual(manifest["installed_version"], "1.2.3")
        self.assertEqual(manifest["protocol_version"], 1)
        self.assertEqual(manifest["install_dir"], str(self.applications))
        with (self.app / "Contents/Info.plist").open("rb") as source:
            info = plistlib.load(source)
        self.assertEqual(info["CFBundleIdentifier"], "com.entomak.entocad")
        self.assertEqual(info["CFBundleVersion"], "1.2.3")
        self.assertNotIn("CFBundleDocumentTypes", info)  # No Finder AppleEvents support yet.
        self.assertTrue((self.app / "Contents/Resources/EntoCAD.icns").is_file())
        self.assertIn("--locked", self.tool_calls("cargo")[0])
        self.assertFalse(self.tool_calls("open"))
        (self.repo / "Cargo.toml").write_text('[workspace.package]\nversion = "1.2.4"\n')
        self.run_installer(MOCK_BINARY="second build")
        self.assertEqual(self.binary.read_text(), "second build")
        self.assertEqual(json.loads(self.manifest.read_text())["installed_version"], "1.2.4")
        self.assert_clean_transaction()

    def test_launch_reopens_literal_drawing_path(self):
        drawing = self.root / 'drawing "quoted" $(not-a-command).dwg'
        drawing.touch()
        self.run_installer("--launch", "--reopen", str(drawing))
        self.assertEqual(self.tool_calls("open"), [["open", "-n", str(self.app), "--args", str(drawing)]])

    def test_failure_keeps_previous_bundle_and_does_not_launch(self):
        self.run_installer()
        original_manifest = self.manifest.read_bytes()
        for fault in ("MOCK_BUILD_FAIL", "MOCK_ICON_FAIL", "MOCK_PUBLISH_FAIL"):
            with self.subTest(fault=fault):
                self.run_installer("--launch", expected=1, MOCK_BINARY="replacement", **{fault: "1"})
                self.assertEqual(self.binary.read_text(), "first build")
                self.assertEqual(self.manifest.read_bytes(), original_manifest)
                self.assertFalse(self.tool_calls("open"))
                self.assert_clean_transaction()
        self.assertIn("simulated Cargo build failure", self.log.read_text())
        self.assertIn("simulated app publication failure", self.log.read_text())
        self.assertEqual(self.tool_calls("osascript"), [["osascript", "-", str(self.log)]] * 3)

    def test_waits_for_old_process_before_relaunch(self):
        child = subprocess.Popen(["sleep", "0.25"])
        reaper = threading.Thread(target=child.wait)
        reaper.start()
        self.run_installer("--wait-pid", str(child.pid), "--launch")
        reaper.join(timeout=5)
        self.assertFalse(reaper.is_alive())
        self.assertIsNotNone(child.returncode)
        self.assertEqual(len(self.tool_calls("open")), 1)

    def test_uninstall_preserves_other_files(self):
        self.run_installer()
        sentinel = self.applications / "other app or drawing.dwg"
        sentinel.write_text("keep this")
        self.run_installer("--uninstall")
        self.assertFalse(self.app.exists())
        self.assertEqual(sentinel.read_text(), "keep this")
        self.assert_clean_transaction()

    def test_refuses_unrelated_destination(self):
        self.app.mkdir(parents=True)
        sentinel = self.app / "user drawing.dwg"
        sentinel.write_text("keep this")
        self.run_installer(expected=1)
        self.run_installer("--uninstall", expected=1)
        self.assertEqual(sentinel.read_text(), "keep this")
        self.assertFalse(self.tool_calls("cargo"))
        self.assert_clean_transaction()

    def test_help_and_validation_do_not_build(self):
        self.assertIn("Usage:", self.run_installer("--help", MOCK_OS="Linux").stdout)
        self.run_installer(expected=1, MOCK_OS="Linux")
        self.run_installer("--wait-pid", "-1", expected=1)
        self.run_installer("--reopen", "relative.dwg", "--launch", expected=1)
        self.run_installer("--launch", "--uninstall", expected=1)
        self.assertFalse(self.tool_calls("cargo"))
        self.assertFalse(self.applications.exists())


if __name__ == "__main__":
    unittest.main()
