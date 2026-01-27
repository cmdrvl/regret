#!/bin/sh
set -eu

info() {
  printf '%s\n' "$*"
}

warn() {
  printf 'warning: %s\n' "$*" >&2
}

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

command_exists() {
  command -v "$1" >/dev/null 2>&1
}

download() {
  url=$1
  dest=$2

  if command_exists curl; then
    curl -fsSL "$url" -o "$dest"
    return 0
  fi

  if command_exists wget; then
    wget -q "$url" -O "$dest"
    return 0
  fi

  fail "curl or wget is required to download $url"
}

fetch() {
  url=$1

  if command_exists curl; then
    curl -fsSL "$url"
    return 0
  fi

  if command_exists wget; then
    wget -q -O - "$url"
    return 0
  fi

  fail "curl or wget is required to fetch $url"
}

sha256_file() {
  file=$1

  if command_exists sha256sum; then
    sha256sum "$file" | awk '{print $1}'
    return 0
  fi

  if command_exists shasum; then
    shasum -a 256 "$file" | awk '{print $1}'
    return 0
  fi

  fail "sha256sum or shasum is required to verify checksums"
}

normalize_version() {
  version=$1
  case "$version" in
    v*) printf '%s' "$version" ;;
    *) printf 'v%s' "$version" ;;
  esac
}

get_latest_version() {
  payload=$(fetch "https://api.github.com/repos/cmdrvl/regret/releases/latest" 2>/dev/null || true)
  version=$(printf '%s' "$payload" \
    | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
    | head -n 1)
  if [ -z "$version" ]; then
    fail "unable to resolve latest release tag"
  fi
  printf '%s' "$version"
}

resolve_install_dir() {
  if [ -n "${REGRET_INSTALL_DIR:-}" ]; then
    printf '%s' "$REGRET_INSTALL_DIR"
    return 0
  fi

  if [ -n "${XDG_BIN_HOME:-}" ]; then
    printf '%s' "$XDG_BIN_HOME"
    return 0
  fi

  if [ -z "${HOME:-}" ]; then
    fail "HOME is not set; set REGRET_INSTALL_DIR instead"
  fi

  printf '%s/.local/bin' "$HOME"
}

detect_target() {
  os=$(uname -s 2>/dev/null || echo unknown)
  arch=$(uname -m 2>/dev/null || echo unknown)

  case "$arch" in
    x86_64|amd64) arch="x86_64" ;;
    arm64|aarch64) arch="aarch64" ;;
    *) fail "unsupported architecture: $arch" ;;
  esac

  case "$os" in
    Linux) printf '%s-unknown-linux-gnu' "$arch" ;;
    Darwin) printf '%s-apple-darwin' "$arch" ;;
    *) fail "unsupported OS: $os" ;;
  esac
}

require_command() {
  name=$1
  if ! command_exists "$name"; then
    fail "required dependency missing: $name"
  fi
}

make_temp_dir() {
  if command_exists mktemp; then
    tmp=$(mktemp -d 2>/dev/null || mktemp -d -t regret-install)
  else
    tmp="${TMPDIR:-/tmp}/regret-install-$$"
    mkdir -p "$tmp"
  fi
  printf '%s' "$tmp"
}

require_command tar
require_command awk
require_command sed
require_command uname
require_command git

version="${REGRET_VERSION:-}"
if [ -z "$version" ]; then
  info "No REGRET_VERSION set; resolving latest release..."
  version=$(get_latest_version)
fi

version=$(normalize_version "$version")
target=$(detect_target)
asset="regret-$version-$target.tar.gz"
base_url="https://github.com/cmdrvl/regret/releases/download/$version"

install_dir=$(resolve_install_dir)
versioned_binary="$install_dir/regret@$version"
active_binary="$install_dir/regret"

tmp_root=$(make_temp_dir)
trap 'rm -rf "$tmp_root"' EXIT INT TERM
archive_path="$tmp_root/$asset"
sha_path="$tmp_root/SHA256SUMS"
sig_path="$tmp_root/SHA256SUMS.sig"
pem_path="$tmp_root/SHA256SUMS.pem"
extract_dir="$tmp_root/extract"

info "Installing regret $version for $target"
info "Install dir: $install_dir"

download "$base_url/$asset" "$archive_path"

skip_verify=0
if [ "${REGRET_NO_VERIFY:-}" = "1" ]; then
  skip_verify=1
  warn "REGRET_NO_VERIFY=1 set; skipping checksum verification"
fi

if [ "$skip_verify" -eq 0 ]; then
  download "$base_url/SHA256SUMS" "$sha_path"

  expected_hash=$(awk -v asset="$asset" '$2 == asset { print $1 }' "$sha_path")
  if [ -z "$expected_hash" ]; then
    fail "checksum for $asset not found in SHA256SUMS"
  fi

  actual_hash=$(sha256_file "$archive_path")
  if [ "$expected_hash" != "$actual_hash" ]; then
    fail "checksum mismatch for $asset"
  fi

  info "Checksum verified."

  if command_exists cosign; then
    if download "$base_url/SHA256SUMS.sig" "$sig_path" && download "$base_url/SHA256SUMS.pem" "$pem_path"; then
      if ! cosign verify-blob \
        --certificate "$pem_path" \
        --signature "$sig_path" \
        --certificate-identity "https://github.com/cmdrvl/regret/.github/workflows/release.yml@refs/tags/$version" \
        --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
        "$sha_path" >/dev/null 2>&1; then
        fail "cosign verification failed for SHA256SUMS"
      fi
      info "Signature verified (cosign)."
    fi
  fi
fi

mkdir -p "$extract_dir"
tar -xzf "$archive_path" -C "$extract_dir"

binary_path="$extract_dir/regret"
if [ ! -f "$binary_path" ]; then
  binary_path=$(find "$extract_dir" -type f -name regret | head -n 1 || true)
fi

if [ -z "$binary_path" ] || [ ! -f "$binary_path" ]; then
  fail "regret binary not found in archive"
fi

mkdir -p "$install_dir"
cp "$binary_path" "$versioned_binary"
chmod 755 "$versioned_binary"

if (cd "$install_dir" && ln -sf "regret@$version" "regret" 2>/dev/null); then
  :
else
  cp "$versioned_binary" "$active_binary"
  chmod 755 "$active_binary"
fi

info "Installed $versioned_binary"
info "Installed $active_binary"

info "Running self-test..."
"$active_binary" --version >/dev/null

doctor_dir="$tmp_root/doctor-repo"
mkdir -p "$doctor_dir"
(
  cd "$doctor_dir"
  git init -q
  "$active_binary" --doctor >/dev/null
)

info "Self-test complete."

if [ "$(uname -s 2>/dev/null || echo)" = "Darwin" ] && command_exists xattr; then
  if xattr -p com.apple.quarantine "$active_binary" >/dev/null 2>&1; then
    warn "macOS quarantine detected. Remove it with:"
    warn "  xattr -d com.apple.quarantine \"$active_binary\""
  fi
fi

info "Install complete."
info "Rollback: ln -sf \"regret@$version\" \"$active_binary\""
