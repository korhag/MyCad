#!/usr/bin/env bash
# EntoCAD per-user install for Linux.
#
# Builds the release binary and adds an application-menu entry, an icon,
# and an Open with entry for DWG and DXF. Re-run after a git pull to update.
#
#   ./packaging/linux/install.sh
#   ./packaging/linux/install.sh --uninstall

set -euo pipefail

APP_ID="entocad"
APP_NAME="EntoCAD"
BIN_DIR="${HOME}/.local/bin"
APP_DIR="${HOME}/.local/share/applications"
ICON_DIR="${HOME}/.local/share/icons/hicolor/256x256/apps"
ICON_THEME="${HOME}/.local/share/icons/hicolor"
BIN_FILE="${BIN_DIR}/${APP_ID}"
DESKTOP_FILE="${APP_DIR}/${APP_ID}.desktop"
ICON_FILE="${ICON_DIR}/${APP_ID}.png"

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

refresh_caches() {
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "${APP_DIR}" || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -f "${ICON_THEME}" >/dev/null 2>&1 || true
    fi
}

uninstall_entocad() {
    rm -f "${BIN_FILE}" "${BIN_DIR}/${APP_ID}.source.json" "${DESKTOP_FILE}" "${ICON_FILE}"
    refresh_caches
    printf '%s was removed. Saved drawings and preferences were left in place.\n' "${APP_NAME}"
}

if [[ "${1:-}" == "--uninstall" ]]; then
    uninstall_entocad
    exit 0
fi
if [[ $# -gt 0 ]]; then
    printf 'Unknown argument: %s\nUsage: %s [--uninstall]\n' "$1" "$0" >&2
    exit 1
fi

printf 'Building %s (release)...\n' "${APP_NAME}"
(
    cd "${ROOT}"
    cargo build --release -p mycad
)

install -d -m 0755 "${BIN_DIR}" "${APP_DIR}" "${ICON_DIR}"
install -m 0755 "${ROOT}/target/release/mycad" "${BIN_FILE}"
install -m 0644 "${ROOT}/resources/icons/entocad-256.png" "${ICON_FILE}"

# The binary path is inserted as literal text. A quote would break the
# Exec line, which is already wrapped in double quotes in the template.
if [[ "${BIN_FILE}" == *\"* ]]; then
    printf 'Install path contains a quote and cannot be written into the desktop file: %s\n' "${BIN_FILE}" >&2
    exit 1
fi
template="$(<"${ROOT}/packaging/linux/entocad.desktop")"
printf '%s%s%s\n' "${template%%@BIN@*}" "${BIN_FILE}" "${template#*@BIN@}" > "${DESKTOP_FILE}"
chmod 0644 "${DESKTOP_FILE}"

refresh_caches

source_file="${BIN_DIR}/${APP_ID}.source.json"
install_script="${ROOT}/packaging/linux/install.sh"
json_escape() {
    printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
}
printf '{"repo":"%s","update_command":"%s"}\n' \
    "$(json_escape "${ROOT}")" \
    "$(json_escape "${install_script}")" > "${source_file}"
chmod 0644 "${source_file}"

printf '\n%s is installed.\n' "${APP_NAME}"
printf 'Application menu: %s\n' "${DESKTOP_FILE}"
printf 'Pin it from the menu or the dock.\n'
printf 'DWG and DXF can be opened with %s from the file manager.\n' "${APP_NAME}"
printf 'Update later by running this script again. Uninstall with --uninstall.\n'
