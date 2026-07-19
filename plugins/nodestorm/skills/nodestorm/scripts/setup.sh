#!/usr/bin/env bash
set -euo pipefail

REPOSITORY="robot-head/nodestorm"
MCP_URL="http://127.0.0.1:4747/mcp"
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
VERSION=$(tr -d '[:space:]' < "$SCRIPT_DIR/../../../VERSION")
DRY_RUN=0
APPROVE_INSTALL=0
APPROVE_LAUNCH=0
SKIP_LAUNCH=0
OS_OVERRIDE=""
ARCH_OVERRIDE=""

usage() {
  echo "Usage: setup.sh [--dry-run] [--os linux|macos] [--arch x64|arm64] [--approve-install] [--approve-launch|--skip-launch]"
}

while (($#)); do
  case "$1" in
    --dry-run) DRY_RUN=1 ;;
    --approve-install) APPROVE_INSTALL=1 ;;
    --approve-launch) APPROVE_LAUNCH=1 ;;
    --skip-launch) SKIP_LAUNCH=1 ;;
    --os) shift; OS_OVERRIDE=${1:-} ;;
    --arch) shift; ARCH_OVERRIDE=${1:-} ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

if ((APPROVE_LAUNCH && SKIP_LAUNCH)); then
  echo "Choose either --approve-launch or --skip-launch." >&2
  exit 2
fi

if [[ -n "$OS_OVERRIDE" ]]; then
  TARGET_OS=$OS_OVERRIDE
else
  case "$(uname -s)" in
    Linux) TARGET_OS="linux" ;;
    Darwin) TARGET_OS="macos" ;;
    *) echo "Unsupported operating system." >&2; exit 1 ;;
  esac
fi

if [[ -n "$ARCH_OVERRIDE" ]]; then
  TARGET_ARCH=$ARCH_OVERRIDE
else
  case "$(uname -m)" in
    x86_64|amd64) TARGET_ARCH="x64" ;;
    arm64|aarch64) TARGET_ARCH="arm64" ;;
    *) echo "Unsupported architecture." >&2; exit 1 ;;
  esac
fi

case "$TARGET_OS/$TARGET_ARCH" in
  linux/x64|linux/arm64) ASSET="nodestorm-v${VERSION}-${TARGET_OS}-${TARGET_ARCH}.tar.gz" ;;
  macos/x64|macos/arm64) ASSET="nodestorm-v${VERSION}-${TARGET_OS}-${TARGET_ARCH}.zip" ;;
  *) echo "Unsupported target: $TARGET_OS/$TARGET_ARCH" >&2; exit 1 ;;
esac

BASE_URL="https://github.com/${REPOSITORY}/releases/download/v${VERSION}"
DOWNLOAD_PROTOCOL="=https"
READINESS_ATTEMPTS=60
if [[ "${NODESTORM_SETUP_TESTING:-0}" == "1" ]]; then
  BASE_URL=${NODESTORM_RELEASE_BASE_URL:-$BASE_URL}
  DOWNLOAD_PROTOCOL=${NODESTORM_DOWNLOAD_PROTOCOL:-$DOWNLOAD_PROTOCOL}
  READINESS_ATTEMPTS=${NODESTORM_READINESS_ATTEMPTS:-$READINESS_ATTEMPTS}
fi
echo "Nodestorm setup target: ${TARGET_OS}/${TARGET_ARCH}"
echo "Pinned artifact: ${ASSET}"
if ((DRY_RUN)); then
  exit 0
fi

confirm() {
  local prompt=$1
  if [[ ! -t 0 ]]; then
    echo "Confirmation required: $prompt" >&2
    return 1
  fi
  read -r -p "$prompt [y/N] " answer
  [[ "$answer" == "y" || "$answer" == "Y" ]]
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "Required trust/runtime tool is missing: $1" >&2
    exit 1
  }
}

port_in_use() {
  (exec 3<>/dev/tcp/127.0.0.1/4747) 2>/dev/null
}

if ((APPROVE_INSTALL == 0)); then
  confirm "Install Nodestorm v$VERSION for $TARGET_OS/$TARGET_ARCH?" || {
    echo "Installation cancelled." >&2
    exit 1
  }
fi

require_command curl
if [[ "$TARGET_OS" == "linux" ]]; then
  require_command sha256sum
  require_command gh
  require_command tar
  require_command ldd
else
  require_command shasum
  require_command unzip
  require_command codesign
  require_command spctl
  require_command open
fi
if port_in_use; then
  echo "Port 4747 is already in use; refusing to install or launch into a conflict." >&2
  exit 1
fi

TEMP_DIR=$(mktemp -d)
trap 'rm -rf -- "$TEMP_DIR"' EXIT
curl --fail --show-error --location --proto "$DOWNLOAD_PROTOCOL" --tlsv1.2 \
  "$BASE_URL/SHA256SUMS" --output "$TEMP_DIR/SHA256SUMS"
curl --fail --show-error --location --proto "$DOWNLOAD_PROTOCOL" --tlsv1.2 \
  "$BASE_URL/$ASSET" --output "$TEMP_DIR/$ASSET"

CHECKSUM_LINE=$(grep -F "  $ASSET" "$TEMP_DIR/SHA256SUMS" || true)
if [[ -z "$CHECKSUM_LINE" ]]; then
  echo "Pinned artifact is absent from SHA256SUMS." >&2
  exit 1
fi
printf '%s\n' "$CHECKSUM_LINE" > "$TEMP_DIR/asset.sha256"

if [[ "$TARGET_OS" == "linux" ]]; then
  (cd "$TEMP_DIR" && sha256sum --check asset.sha256)
  gh attestation verify "$TEMP_DIR/$ASSET" --repo "$REPOSITORY"
  tar -xzf "$TEMP_DIR/$ASSET" -C "$TEMP_DIR"
  STAGED_BINARY="$TEMP_DIR/nodestorm"
  [[ -x "$STAGED_BINARY" ]] || { echo "Release archive has no executable nodestorm binary." >&2; exit 1; }
  LDD_OUTPUT=$(ldd "$STAGED_BINARY" 2>&1) || {
    echo "Unable to inspect GTK/WebKitGTK runtime dependencies." >&2
    exit 1
  }
  if grep -q "not found" <<<"$LDD_OUTPUT"; then
    echo "Missing Linux runtime libraries (GTK/WebKitGTK):" >&2
    grep "not found" <<<"$LDD_OUTPUT" >&2
    exit 1
  fi
  "$STAGED_BINARY" --version | grep -Fx "nodestorm ${VERSION}" >/dev/null || {
    echo "Downloaded binary version does not match $VERSION." >&2
    exit 1
  }
  if [[ "${XDG_DATA_HOME:-}" == /* && "$XDG_DATA_HOME" != *"="* ]]; then
    DATA_HOME=$XDG_DATA_HOME
  else
    DATA_HOME="$HOME/.local/share"
  fi
  for size in 48 128 256 512; do
    staged_icon="$TEMP_DIR/icons/${size}x${size}/nodestorm.png"
    [[ -f "$staged_icon" ]] || { echo "Release archive has no ${size}px launcher icon." >&2; exit 1; }
  done

  INSTALL_DIR="$DATA_HOME/nodestorm/${VERSION}"
  desktop_exec="$INSTALL_DIR/nodestorm"
  if [[ "$desktop_exec" != /* || "$desktop_exec" == *"="* ]]; then
    echo "Desktop executable path cannot be represented in a desktop entry." >&2
    exit 1
  fi
  mkdir -p "$INSTALL_DIR"
  install -m 0755 "$STAGED_BINARY" "$INSTALL_DIR/nodestorm"
  for size in 48 128 256 512; do
    staged_icon="$TEMP_DIR/icons/${size}x${size}/nodestorm.png"
    icon_dir="$DATA_HOME/icons/hicolor/${size}x${size}/apps"
    mkdir -p "$icon_dir"
    install -m 0644 "$staged_icon" "$icon_dir/nodestorm.png"
  done

  desktop_dir="$DATA_HOME/applications"
  mkdir -p "$desktop_dir"
  desktop_exec=${desktop_exec//\\/\\\\}
  desktop_exec=${desktop_exec//\"/\\\"}
  desktop_exec=${desktop_exec//\$/\\$}
  desktop_exec=${desktop_exec//\`/\\\`}
  desktop_exec=${desktop_exec//%/%%}
  desktop_exec=${desktop_exec//\\/\\\\}
  {
    printf '[Desktop Entry]\nType=Application\nVersion=1.0\n'
    printf 'Name=Nodestorm\nComment=Visual architecture brainstorming\n'
    printf 'Exec="%s"\nIcon=nodestorm\nTerminal=false\nCategories=Development;\n' "$desktop_exec"
  } > "$desktop_dir/nodestorm.desktop"
  chmod 0644 "$desktop_dir/nodestorm.desktop"
  LAUNCH_COMMAND=("$INSTALL_DIR/nodestorm")
else
  (cd "$TEMP_DIR" && shasum -a 256 --check asset.sha256)
  unzip -q "$TEMP_DIR/$ASSET" -d "$TEMP_DIR"
  STAGED_APP="$TEMP_DIR/Nodestorm.app"
  [[ -d "$STAGED_APP" ]] || { echo "Release archive has no Nodestorm.app bundle." >&2; exit 1; }
  codesign --verify --deep --strict --verbose=2 "$STAGED_APP"
  spctl --assess --type execute --verbose=2 "$STAGED_APP"
  STAGED_BINARY="$STAGED_APP/Contents/MacOS/nodestorm"
  "$STAGED_BINARY" --version | grep -Fx "nodestorm ${VERSION}" >/dev/null || {
    echo "Downloaded app version does not match $VERSION." >&2
    exit 1
  }
  INSTALL_DIR="${HOME}/Applications"
  INSTALL_APP="$INSTALL_DIR/Nodestorm.app"
  mkdir -p "$INSTALL_DIR"
  if [[ -e "$INSTALL_APP" ]]; then
    BACKUP_APP="$INSTALL_DIR/Nodestorm.app.backup-$(date +%Y%m%d%H%M%S)"
    mv "$INSTALL_APP" "$BACKUP_APP"
    echo "Previous app preserved at $BACKUP_APP"
  fi
  cp -R "$STAGED_APP" "$INSTALL_APP"
  codesign --verify --deep --strict --verbose=2 "$INSTALL_APP"
  spctl --assess --type execute --verbose=2 "$INSTALL_APP"
  LAUNCH_COMMAND=(open -a "$INSTALL_APP")
fi

echo "Installed trusted Nodestorm v$VERSION without sudo or PATH changes."
if ((SKIP_LAUNCH)); then
  echo "Installed; launch skipped."
  exit 0
fi
if ((APPROVE_LAUNCH == 0)); then
  confirm "Launch Nodestorm now?" || {
    echo "Installed; launch skipped."
    exit 0
  }
fi

if port_in_use; then
  echo "Port 4747 became unavailable before launch." >&2
  exit 1
fi

if [[ "$TARGET_OS" == "linux" ]]; then
  LOG_FILE="$DATA_HOME/nodestorm/nodestorm.log"
  "${LAUNCH_COMMAND[@]}" >"$LOG_FILE" 2>&1 &
else
  "${LAUNCH_COMMAND[@]}"
fi

INITIALIZE='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"nodestorm-setup","version":"0.9.0"}}}'
for ((_attempt = 1; _attempt <= READINESS_ATTEMPTS; _attempt++)); do
  RESPONSE=$(curl --silent --show-error --max-time 2 \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json, text/event-stream' \
    --data "$INITIALIZE" "$MCP_URL" 2>/dev/null || true)
  if grep -q '"serverInfo"' <<<"$RESPONSE"; then
    echo "Nodestorm MCP is ready at $MCP_URL"
    exit 0
  fi
  sleep 1
done

echo "Nodestorm launched but MCP readiness timed out after ${READINESS_ATTEMPTS} seconds." >&2
exit 1
