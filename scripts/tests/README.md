# Installer smoke tests

Run from the repository root:

```bash
python3 -B -m unittest discover -s scripts/tests -p 'test_install_*.py' -v
pwsh -NoLogo -NoProfile -File scripts/tests/test_install_windows.ps1
```

The Python tests run on Linux with mocked Cargo/native macOS commands and
explicit temporary installation directories. They exercise publication,
rollback, launcher arguments, custom paths, process waiting and uninstall.
The Linux launcher test also uses `gio` and `desktop-file-validate` when present.
No test installs EntoCAD in your real user profile.

The PowerShell test parses the installer and extracts its functions without
running installation setup. It uses temporary files and a mocked Cargo command;
registry and shortcut creation are mocked. PowerShell 7 runs this smoke on Linux
or Windows; the installer also targets Windows PowerShell 5.1.

Before a platform release, also test the real installer on Windows/macOS:
create the menu/Dock shortcut, open the app, update from a changed checkout,
confirm the saved drawing reopens, and uninstall. Mock tests cannot validate
Windows COM/registry or macOS LaunchServices, native libraries and signing.
