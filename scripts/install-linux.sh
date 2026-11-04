#!/usr/bin/env bash
# Per-user Linux installation. Re-run from the same checkout after git pull.
set -Eeuo pipefail

usage() {
    cat <<'EOF'
Usage: bash scripts/install-linux.sh [options]

Build EntoCAD and install a stable application-menu launcher for this user.
Existing menu/dock pins launch the newly installed version after an update.
Rust/Cargo, Python 3, a C compiler, pkg-config and libclang are required; see README.md
for your distribution's build packages. No sudo or automatic toolchain setup.

  --launch           Open EntoCAD after a successful installation
  --reopen FILE      Open an absolute drawing path (requires --launch)
  --wait-pid PID     Wait up to 120 seconds for the old app to exit
  --prefix DIR       Absolute installation prefix (default: ~/.local)
  --data-dir DIR     Absolute menu/icon directory (default: XDG_DATA_HOME
                     or PREFIX/share)
  --log-file FILE    Absolute installation log (default: XDG_STATE_HOME/
                     entocad/install.log or ~/.local/state/entocad/install.log)
  --uninstall        Remove this installation; keep drawings and preferences
  --help             Show this help

Installation builds the checked-out source with Cargo.lock. It never pulls git.
EOF
}

fail() { printf 'EntoCAD installer: %s\n' "$*" >&2; exit 1; }
require_value() { [[ $# -ge 2 && -n "$2" ]] || fail "$1 requires a value."; }
absolute_path() { [[ "$2" == /* ]] || fail "$1 must be an absolute path: $2"; }

PREFIX="${HOME:?HOME is not set}/.local"
DATA_DIR=""
LOG_FILE=""
WAIT_PID=""
REOPEN=""
LAUNCH=0
UNINSTALL=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --help|-h) usage; exit 0 ;;
        --launch) LAUNCH=1; shift ;;
        --uninstall) UNINSTALL=1; shift ;;
        --prefix) require_value "$@"; PREFIX="$2"; shift 2 ;;
        --data-dir) require_value "$@"; DATA_DIR="$2"; shift 2 ;;
        --log-file) require_value "$@"; LOG_FILE="$2"; shift 2 ;;
        --wait-pid) require_value "$@"; WAIT_PID="$2"; shift 2 ;;
        --reopen) require_value "$@"; REOPEN="$2"; shift 2 ;;
        *) fail "Unknown argument: $1 (use --help)." ;;
    esac
done
[[ "$(uname -s)" == Linux ]] || fail 'Use the installer for your operating system.'
absolute_path '--prefix' "$PREFIX"
DATA_DIR="${DATA_DIR:-${XDG_DATA_HOME:-$PREFIX/share}}"
LOG_FILE="${LOG_FILE:-${XDG_STATE_HOME:-$HOME/.local/state}/entocad/install.log}"
absolute_path '--data-dir / XDG_DATA_HOME' "$DATA_DIR"
absolute_path '--log-file / XDG_STATE_HOME' "$LOG_FILE"
if [[ -n "$WAIT_PID" ]]; then
    [[ "$WAIT_PID" =~ ^[1-9][0-9]*$ && ${#WAIT_PID} -le 10 && "$WAIT_PID" != "$$" ]] || fail 'Invalid --wait-pid.'
fi
if [[ -n "$REOPEN" ]]; then
    absolute_path '--reopen' "$REOPEN"
    [[ "$LAUNCH" == 1 ]] || fail '--reopen requires --launch.'
fi
[[ "$UNINSTALL" == 0 || ( "$LAUNCH" == 0 && -z "$REOPEN" ) ]] || fail '--uninstall cannot be combined with --launch or --reopen.'

SCRIPT_FILE="$(readlink -f -- "${BASH_SOURCE[0]}")"
ROOT="$(cd -- "$(dirname -- "$SCRIPT_FILE")/.." && pwd -P)"
mkdir -p -- "$(dirname -- "$LOG_FILE")"
touch -- "$LOG_FILE" || fail "Cannot write installation log: $LOG_FILE"
# The updater captures shell startup output separately; this main log also
# records normal terminal installations and their complete build diagnostics.
exec > >(tee -a -- "$LOG_FILE") 2>&1
printf '\nEntoCAD Linux installer — %s\nLog: %s\n' "$(date -u '+%Y-%m-%d %H:%M:%S UTC')" "$LOG_FILE"

STAGE_DIRS=()
TARGETS=()
STAGED_FILES=()
BACKUPS=()
REPLACED=0
COMMITTED=0
cleanup() {
    local status=$? index rollback_failed=0
    trap - EXIT
    set +e
    if [[ "$status" != 0 ]]; then
        if [[ "$COMMITTED" == 0 ]]; then
            for ((index=REPLACED-1; index>=0; index--)); do
                if [[ -e "${BACKUPS[index]}" || -L "${BACKUPS[index]}" ]]; then
                    if ! mv -fT -- "${BACKUPS[index]}" "${TARGETS[index]}"; then
                        printf 'Could not restore %s; backup retained at %s\n' "${TARGETS[index]}" "${BACKUPS[index]}" >&2
                        rollback_failed=1
                    fi
                else
                    rm -f -- "${TARGETS[index]}"
                fi
            done
        fi
        printf '\nInstallation failed (exit %s). Details: %s\n' "$status" "$LOG_FILE" >&2
        printf 'Resolve the reported error and run this script again.\n' >&2
        if [[ "$LAUNCH" == 1 ]] && command -v notify-send >/dev/null 2>&1; then
            timeout 5s notify-send -a EntoCAD -u critical 'EntoCAD update failed' "The installation did not complete. See $LOG_FILE" || true
        fi
    fi
    if [[ "$rollback_failed" == 0 ]]; then
        for index in "${STAGE_DIRS[@]}"; do rm -rf -- "$index"; done
    fi
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

command -v flock >/dev/null 2>&1 || fail 'Install util-linux (flock) using your distribution package manager.'
mkdir -p -- "$PREFIX/bin" "$DATA_DIR"
PREFIX="$(cd -- "$PREFIX" && pwd -P)"
DATA_DIR="$(cd -- "$DATA_DIR" && pwd -P)"
BIN_FILE="$PREFIX/bin/entocad"
SOURCE_FILE="$PREFIX/bin/entocad.source.json"
APP_DIR="$DATA_DIR/applications"
ICON_THEME="$DATA_DIR/icons/hicolor"
ICON_DIR="$ICON_THEME/256x256/apps"
DESKTOP_FILE="$APP_DIR/entocad.desktop"
ICON_FILE="$ICON_DIR/entocad.png"
exec 9> "$PREFIX/bin/.entocad-install.lock"
flock -n 9 || fail 'Another EntoCAD installer is running for this prefix. Try again after it finishes.'

wait_for_app() {
    [[ -n "$WAIT_PID" ]] || return 0
    printf 'Waiting for EntoCAD (PID %s) to close...\n' "$WAIT_PID"
    local deadline=$((SECONDS + 120))
    while kill -0 "$WAIT_PID" 2>/dev/null; do
        # An exited child can remain a zombie until its parent reaps it.
        if [[ -r "/proc/$WAIT_PID/status" ]] && [[ "$(sed -n 's/^State:[[:space:]]*\([A-Z]\).*/\1/p' "/proc/$WAIT_PID/status" 2>/dev/null)" == Z ]]; then
            return 0
        fi
        (( SECONDS < deadline )) || fail 'The previous app is still running after 120 seconds. Close it and retry; it has not been terminated.'
        sleep 0.2
    done
}

refresh_caches() {
    if command -v update-desktop-database >/dev/null 2>&1 && [[ -d "$APP_DIR" ]]; then
        update-desktop-database "$APP_DIR" || printf 'Desktop cache refresh failed; your desktop may need to refresh its application menu.\n'
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1 && [[ -d "$ICON_THEME" ]]; then
        gtk-update-icon-cache -f -t "$ICON_THEME" >/dev/null 2>&1 || true
    fi
}

if [[ "$UNINSTALL" == 1 ]]; then
    wait_for_app
    rm -f -- "$BIN_FILE" "$SOURCE_FILE" "$DESKTOP_FILE" "$ICON_FILE"
    refresh_caches
    printf 'EntoCAD was removed. Saved drawings and preferences were left in place.\n'
    exit 0
fi

# Desktop sessions often omit Cargo's directory from PATH.
if ! command -v cargo >/dev/null 2>&1 && [[ -x "${CARGO_HOME:-$HOME/.cargo}/bin/cargo" ]]; then
    export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
fi
command -v cargo >/dev/null 2>&1 || fail 'Rust/Cargo is missing. Install Rust from https://rustup.rs, open a new terminal, and retry. See README.md for Linux build packages.'
command -v python3 >/dev/null 2>&1 || fail 'Python 3 is required to read Cargo build artifacts. Install python3 using your distribution package manager and retry.'
[[ -f "$ROOT/Cargo.lock" && -f "$ROOT/resources/icons/entocad-256.png" ]] || fail "Incomplete repository checkout: $ROOT"
VERSION="$(awk '/^\[workspace.package\]/{found=1;next} found && /^\[/{exit} found && /^version[[:space:]]*=/{gsub(/^[^"]*"|"[^\n]*$/, ""); print; exit}' "$ROOT/Cargo.toml")"
[[ -n "$VERSION" ]] || fail 'Cannot read the workspace version from Cargo.toml.'
COMMIT="$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || true)"
BUILD_STAGE="$(mktemp -d -- "$PREFIX/bin/.entocad-build.XXXXXX")"
STAGE_DIRS+=("$BUILD_STAGE")
printf 'Building EntoCAD %s (release). This may take several minutes...\n' "$VERSION"
if ! (cd -- "$ROOT" && cargo build --release -p mycad --locked --message-format=json-render-diagnostics > "$BUILD_STAGE/artifacts.jsonl"); then
    fail 'Cargo build failed; the installed app was not changed. Check Rust and the Linux build packages in README.md (C compiler, pkg-config, clang/libclang, GTK and XCB development libraries).'
fi
# Use Cargo's actual artifact, respecting CARGO_TARGET_DIR and .cargo/config.toml
# native build.target/target-dir settings without guessing an output directory.
BUILT_BINARY="$(python3 - "$BUILD_STAGE/artifacts.jsonl" <<'PYTHON'
import json
import pathlib
import sys

artifact = None
with open(sys.argv[1], encoding="utf-8") as records:
    for line in records:
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        target = record.get("target", {})
        if (record.get("reason") == "compiler-artifact"
                and target.get("name") == "mycad"
                and "bin" in target.get("kind", [])
                and record.get("executable")):
            artifact = pathlib.Path(record["executable"])
if artifact is None or not artifact.is_absolute() or not artifact.is_file():
    sys.exit("Cargo did not report the mycad executable. Check your build configuration.")
print(artifact)
PYTHON
)"
[[ -x "$BUILT_BINARY" ]] || fail "Cargo's executable is not runnable: $BUILT_BINARY"

# Desktop entries have two escaping layers: Exec quoting, then string escaping.
# Percent is a desktop field code even inside quotes, and must be doubled.
desktop_argument() {
    local value="$1"
    value="${value//\\/\\\\}"
    value="${value//\"/\\\"}"
    value="${value//\$/\\\$}"
    value="${value//\`/\\\`}"
    value="${value//%/%%}"
    value="${value//\\/\\\\}"
    value="${value//$'\n'/\\n}"
    value="${value//$'\r'/\\r}"
    value="${value//$'\t'/\\t}"
    printf '"%s"' "$value"
}

stage_file() {
    local target="$1" directory
    directory="$(mktemp -d -- "$(dirname -- "$target")/.entocad-install.XXXXXX")"
    STAGE_DIRS+=("$directory")
    TARGETS+=("$target")
    STAGED_FILES+=("$directory/new")
    BACKUPS+=("$directory/old")
    if [[ -e "$target" || -L "$target" ]]; then
        [[ ! -d "$target" ]] || fail "Install destination is a directory: $target"
        cp -a -- "$target" "$directory/old"
    fi
}
mkdir -p -- "$APP_DIR" "$ICON_DIR"
stage_file "$BIN_FILE"
install -m 0755 -- "$BUILT_BINARY" "${STAGED_FILES[0]}"
stage_file "$SOURCE_FILE"
python3 - "$ROOT" "$SCRIPT_FILE" "$VERSION" "$COMMIT" "$PREFIX" "$DATA_DIR" > "${STAGED_FILES[1]}" <<'PYTHON'
import json
import sys

keys = ("repo", "update_command", "installed_version", "installed_commit", "install_prefix", "data_dir")
print(json.dumps({"protocol_version": 1, **dict(zip(keys, sys.argv[1:]))}))
PYTHON
chmod 0644 -- "${STAGED_FILES[1]}"
stage_file "$ICON_FILE"
install -m 0644 -- "$ROOT/resources/icons/entocad-256.png" "${STAGED_FILES[2]}"
stage_file "$DESKTOP_FILE"
{
    # GLib checks the first word before expanding %% field escapes. Using env
    # lets binary paths containing a literal percent work on GNOME as well.
    printf '[Desktop Entry]\nType=Application\nVersion=1.0\nName=EntoCAD\nGenericName=2D CAD\nComment=Open and edit DWG and DXF drawings\nExec=/usr/bin/env -- '
    desktop_argument "$BIN_FILE"
    printf ' %%f\nIcon=entocad\nStartupWMClass=entocad\nCategories=Graphics;Engineering;\nMimeType=image/vnd.dwg;image/vnd.dxf;\nTerminal=false\nStartupNotify=true\n'
} > "${STAGED_FILES[3]}"
chmod 0644 -- "${STAGED_FILES[3]}"
wait_for_app
for ((index=0; index<${#TARGETS[@]}; index++)); do
    # Stage and destination share a filesystem; inode replacement also works
    # while an independently launched older copy is still running.
    REPLACED=$((index + 1))
    mv -fT -- "${STAGED_FILES[index]}" "${TARGETS[index]}"
done
COMMITTED=1
refresh_caches
printf '\nEntoCAD %s is installed.\nApplication menu: %s\n' "$VERSION" "$DESKTOP_FILE"
printf 'Pin EntoCAD from the application menu. Existing pins will use this installation after future updates.\n'
printf 'Update by pulling git and running this script again. Uninstall with --uninstall and the same path options.\n'
if [[ "$LAUNCH" == 1 ]]; then
    launch_args=()
    [[ -z "$REOPEN" ]] || launch_args+=("$REOPEN")
    printf 'Opening EntoCAD...\n'
    # Release the installer lock in the child and detach its standard streams.
    nohup "$BIN_FILE" "${launch_args[@]}" </dev/null >> "$LOG_FILE" 2>&1 9>&- &
fi
