#!/usr/bin/env bash
# Build and install the current checkout. No sudo, network updater, or Dock edits.
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: bash scripts/install-macos.sh [options]

Build EntoCAD and install ~/Applications/EntoCAD.app. Re-run after git pull;
existing Finder and Dock shortcuts continue to open the same app bundle.

  --install-dir ABS    Directory containing EntoCAD.app (default ~/Applications)
  --launch             Start the installed application after success
  --reopen ABS         Reopen a drawing on launch (requires --launch)
  --wait-pid PID       Wait up to 120 seconds for the old application to close
  --log-file ABS       Append build/install output to this file
  --uninstall          Remove the installed app; keep drawings and preferences
  --help               Show this help

Requires Rust (cargo), the Xcode Command Line Tools, and macOS. Build failures
leave the previous installed app in place. No dependencies are downloaded by
this script other than Cargo's normal locked project dependencies.
EOF
}

fail() { printf 'Error: %s\n' "$*" >&2; exit 1; }
require_value() {
    [[ $# -ge 2 && -n "$2" ]] || fail "$1 requires a value."
}

install_dir="${HOME}/Applications"
log_file="${HOME}/Library/Logs/EntoCAD/install.log"
wait_pid=""
reopen=""
launch=0
uninstall=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --install-dir) require_value "$@"; install_dir="$2"; shift 2 ;;
        --log-file) require_value "$@"; log_file="$2"; shift 2 ;;
        --wait-pid) require_value "$@"; wait_pid="$2"; shift 2 ;;
        --reopen) require_value "$@"; reopen="$2"; shift 2 ;;
        --launch) launch=1; shift ;;
        --uninstall) uninstall=1; shift ;;
        --help|-h) usage; exit 0 ;;
        *) fail "Unknown argument: $1. Use --help for usage." ;;
    esac
done

[[ "$install_dir" == /* && "$log_file" == /* ]] || fail "Install and log paths must be absolute."
[[ -z "$reopen" || ( "$reopen" == /* && "$launch" == 1 ) ]] || fail "--reopen requires an absolute drawing path and --launch."
[[ "$uninstall" == 0 || ( "$launch" == 0 && -z "$reopen" ) ]] || fail "--uninstall cannot be combined with --launch or --reopen."
if [[ -n "$wait_pid" ]]; then
    [[ "$wait_pid" =~ ^[1-9][0-9]*$ && "$wait_pid" != "$$" ]] || fail "--wait-pid requires another process's positive PID."
fi
[[ "$(uname -s)" == Darwin ]] || fail "This installer requires macOS."

# Finder/Dock launches do not load the user's shell startup files.
for tool_dir in "${CARGO_HOME:-${HOME}/.cargo}/bin" /opt/homebrew/bin /usr/local/bin; do
    if [[ -d "$tool_dir" ]]; then PATH="$PATH:$tool_dir"; fi
done
export PATH

mkdir -p "$(dirname "$log_file")"
touch "$log_file"
exec > >(tee -a "$log_file") 2>&1
printf '\nEntoCAD installer — %s\nLog: %s\n' "$(date '+%Y-%m-%d %H:%M:%S %z')" "$log_file"

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
bundle_id="com.entomak.entocad"
lsregister="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
stage=""
lock=""
previous_moved=0
published=0

cleanup() {
    status=$?
    trap - EXIT
    set +e
    # A signal can arrive immediately after mv, before its next assignment.
    if [[ -n "$stage" && -d "$app_bundle" && ! -e "$stage/EntoCAD.app" && "$previous_moved" == 1 ]]; then
        published=1
    fi
    if [[ "$previous_moved" == 1 && "$published" == 0 && -d "$stage/previous.app" ]]; then
        if ! mv "$stage/previous.app" "$app_bundle"; then
            printf 'Could not restore the previous app. It remains at: %s/previous.app\n' "$stage" >&2
            stage=""
            status=1
        fi
    fi
    if [[ -n "$stage" ]]; then rm -rf "$stage"; fi
    if [[ -n "$lock" ]]; then rm -f "$lock/pid"; rmdir "$lock" || true; fi
    if [[ "$status" != 0 ]]; then
        if [[ "$published" == 1 ]]; then
            printf 'EntoCAD is installed, but a final launch step failed. See the log: %s\n' "$log_file" >&2
        else
            printf 'Installation did not complete. See the log: %s\n' "$log_file" >&2
        fi
        if [[ "$launch" == 1 ]] && command -v osascript >/dev/null 2>&1; then
            choice="$(osascript - "$log_file" <<'APPLESCRIPT'
on run argv
    set logPath to item 1 of argv
    set dialogResult to display dialog ("EntoCAD could not finish updating or restarting. Your drawings and preferences were kept. For details, open the installation log:" & return & return & logPath) with title "EntoCAD update" buttons {"Close", "Open log"} default button "Open log" with icon caution
    return button returned of dialogResult
end run
APPLESCRIPT
)"
            if [[ "$choice" == "Open log" ]]; then open -t "$log_file" || true; fi
        fi
    fi
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

command -v plutil >/dev/null 2>&1 || fail "The macOS plutil utility is required."
mkdir -p "$install_dir"
install_dir="$(cd "$install_dir" && pwd -P)"
app_bundle="$install_dir/EntoCAD.app"
lock_path="$install_dir/.entocad-install.lock"
if ! mkdir "$lock_path" 2>/dev/null; then
    fail "Another installation may be running. If it has stopped, remove $lock_path and retry."
fi
lock="$lock_path"
printf '%s\n' "$$" > "$lock/pid"

# Refuse to replace or remove an unrelated directory at the destination.
if [[ -e "$app_bundle" || -L "$app_bundle" ]]; then
    [[ ! -L "$app_bundle" && -d "$app_bundle" ]] || fail "The destination is not a regular app bundle: $app_bundle"
    existing_id="$(plutil -extract CFBundleIdentifier raw -o - "$app_bundle/Contents/Info.plist" 2>/dev/null || true)"
    [[ "$existing_id" == "$bundle_id" ]] || fail "The destination is not an EntoCAD bundle: $app_bundle"
fi

if [[ -n "$wait_pid" ]]; then
    printf 'Waiting for EntoCAD (PID %s) to close...\n' "$wait_pid"
    seconds=0
    while kill -0 "$wait_pid" 2>/dev/null; do
        [[ "$seconds" -lt 120 ]] || fail "EntoCAD is still running. Close it and run the installer again."
        sleep 1
        seconds=$((seconds + 1))
    done
fi

if [[ "$uninstall" == 1 ]]; then
    if [[ -d "$app_bundle" ]]; then
        if [[ -x "$lsregister" ]]; then "$lsregister" -u "$app_bundle" || true; fi
        rm -rf "$app_bundle"
    fi
    printf 'EntoCAD was removed. Saved drawings and preferences were left in place.\n'
    exit 0
fi

for tool in cargo sips iconutil; do
    command -v "$tool" >/dev/null 2>&1 || fail "Required tool '$tool' was not found. Install Rust and the Xcode Command Line Tools first."
done

stage="$(mktemp -d "$install_dir/.entocad-install.XXXXXX")"
printf 'Building EntoCAD (release)...\n'
(
    cd "$repo"
    cargo build --release --locked -p mycad --message-format=json-render-diagnostics > "$stage/build.jsonl"
)

# Cargo supplies the actual binary path, including CARGO_TARGET_DIR and .cargo
# configuration. Parse native JSON with plutil; Python is not required on macOS.
executable=""
while IFS= read -r artifact; do
    case "$artifact" in
        *'"compiler-artifact"'*)
            printf '%s\n' "$artifact" > "$stage/artifact.json"
            target="$(plutil -extract target.name raw -o - "$stage/artifact.json" 2>/dev/null || true)"
            kind="$(plutil -extract target.kind.0 raw -o - "$stage/artifact.json" 2>/dev/null || true)"
            if [[ "$target" == mycad && "$kind" == bin ]]; then
                executable="$(plutil -extract executable raw -o - "$stage/artifact.json")"
            fi
            ;;
    esac
done < "$stage/build.jsonl"
[[ -n "$executable" && -f "$executable" && -x "$executable" ]] || fail "Cargo did not report an executable for mycad."

version="$(awk '
    /^\[workspace\.package\]/ { package = 1; next }
    /^\[/ { package = 0 }
    package && /^[[:space:]]*version[[:space:]]*=/ {
        sub(/^[^"]*"/, ""); sub(/".*$/, ""); print; exit
    }
' "$repo/Cargo.toml")"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.+-]+)?$ ]] || fail "Cannot read the workspace version."
bundle_version="${version%%[-+]*}"
new_bundle="$stage/EntoCAD.app"
mkdir -p "$new_bundle/Contents/MacOS" "$new_bundle/Contents/Resources" "$stage/EntoCAD.iconset"
install -m 0755 "$executable" "$new_bundle/Contents/MacOS/EntoCAD"
install -m 0644 "$repo/packaging/macos/Info.plist" "$new_bundle/Contents/Info.plist"
plutil -replace CFBundleShortVersionString -string "$bundle_version" "$new_bundle/Contents/Info.plist"
plutil -replace CFBundleVersion -string "$bundle_version" "$new_bundle/Contents/Info.plist"
plutil -lint "$new_bundle/Contents/Info.plist"

for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$repo/resources/icons/entocad-256.png" --out "$stage/EntoCAD.iconset/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z "$double" "$double" "$repo/resources/icons/entocad-256.png" --out "$stage/EntoCAD.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns -o "$new_bundle/Contents/Resources/EntoCAD.icns" "$stage/EntoCAD.iconset"

printf '<?xml version="1.0"?><plist version="1.0"><dict/></plist>\n' > "$stage/source.plist"
plutil -insert repo -string "$repo" "$stage/source.plist"
plutil -insert update_command -string "$repo/scripts/install-macos.sh" "$stage/source.plist"
plutil -insert protocol_version -integer 1 "$stage/source.plist"
plutil -insert installed_version -string "$version" "$stage/source.plist"
plutil -insert install_dir -string "$install_dir" "$stage/source.plist"
installed_commit="$(git -C "$repo" rev-parse HEAD 2>/dev/null || true)"
if [[ "$installed_commit" =~ ^[0-9a-f]{40,64}$ ]]; then
    plutil -insert installed_commit -string "$installed_commit" "$stage/source.plist"
fi
plutil -convert json -o "$new_bundle/Contents/MacOS/EntoCAD.source.json" "$stage/source.plist"
chmod 0644 "$new_bundle/Contents/MacOS/EntoCAD.source.json"

# Staging and backups share the destination filesystem. Any failed publication
# restores the previous bundle; compilation and icon generation never touch it.
if [[ -d "$app_bundle" ]]; then
    previous_moved=1
    mv "$app_bundle" "$stage/previous.app"
fi
mv "$new_bundle" "$app_bundle"
published=1
if [[ -x "$lsregister" ]]; then
    "$lsregister" -f "$app_bundle" || printf 'macOS application registration could not be refreshed. Open EntoCAD.app once from Finder.\n' >&2
fi
printf '\nEntoCAD %s is installed: %s\n' "$version" "$app_bundle"
printf 'Open it from Finder, or drag the app to the Dock to keep a shortcut.\n'
printf 'Re-run this script after git pull to update that same app.\n'
if [[ "$launch" == 1 ]]; then
    if [[ -n "$reopen" ]]; then
        open -n "$app_bundle" --args "$reopen"
    else
        open -n "$app_bundle"
    fi
fi
