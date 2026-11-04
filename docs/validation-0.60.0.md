# EntoCAD 0.60.0 validation

Validated in the Linux cloud workspace on 2026-11-04.

## Automated checks

- `cargo test --workspace --offline`: 895 tests passed, with zero compiler
  warnings. The AutoCAD/ODA audit hooks return early because those external
  programs are not installed; this does not certify AutoCAD fidelity.
- After the final native-snapshot filename fix,
  `cargo test --workspace --bins --locked --offline`: all 362 application tests
  passed, including update guards, installer handoff errors, pending Properties,
  semantic versions, revision detection, manifest migration, platform arguments
  and relative/native drawing reopen paths.
- `cargo build -p mycad --locked --offline`: passed with zero warnings.
- Python installer smoke suite: 19 tests passed (12 Linux, 7 mocked macOS).
  Covered stable paths, Cargo artifact discovery, menu-launch PATH, literal
  special characters, replacement rollback, failed builds, process waiting,
  concurrent installation, reopen arguments and uninstall preservation. Linux
  desktop entries were also exercised with `gio` and `desktop-file-validate`.
- Portable PowerShell 7.5.2 parser/function smoke passed for the Windows
  installer and legacy forwarding entry point. Covered argument quoting, fresh
  installation, failed publication cleanup, upgrade rollback, build failure,
  manifest creation, locked Cargo arguments and process waiting.
- `bash -n` for Linux/macOS installers and the legacy Linux shim, plus
  `git diff --check`: passed.
- `--perf-baseline` on the 30-entity DWG smoke fixture: pointer/selection/lookup
  samples at most 0.17 ms; edit/refresh samples at most 0.20 ms, below the
  16 ms preferred and 33 ms maximum budgets. This is a small regression fixture,
  not a claim about every production drawing.

## Desktop update smoke

An isolated Xvfb/software-GL session used a temporary installed copy, checkout,
XDG directories and fake Cargo artifact output. The real Linux installer and
real app were used; the Cargo build step reused the verified debug executable.
This exercises the update handoff without installing into the user's profile
or rebuilding all dependencies in release mode.

The app detected a higher checkout version, displayed **Update and restart**,
closed successfully, waited for the old process through the installer, updated
the same installed path and launched again. The saved native drawing, initially
opened with a relative filename containing spaces, reopened with its geometry
and correct filename. Logs and the new source manifest remained in the temporary
installation directories.

## Platform limits

Windows COM shortcuts, registry/Open With integration, executable locking and
PowerShell 5.1 execution still require a native Windows smoke test. macOS bundle
launch, native plutil/icon generation, LaunchServices and Dock integration still
require native macOS testing. macOS Finder document associations are deliberately
absent until the app handles document-open AppleEvents; use File → Open.

These installers build local source and do not perform Git pulls. Taskbar/Dock
pinning is user managed. Build failures retain the installed application and
record recovery logs; installing prerequisites remains a one-time user step.
