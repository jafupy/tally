#!/usr/bin/env sh
set -eu

base_url="${TALLY_BASE_URL:-https://jafupy.com/tally/bin}"
install_dir="${TALLY_INSTALL_DIR:-$HOME/.local/bin}"
bin="$install_dir/tally"

say() {
  printf '%s\n' "$*"
}

case "$(uname -s)" in
  Darwin) os="macos" ;;
  Linux) os="linux" ;;
  *)
    say "unsupported OS: $(uname -s)"
    exit 1
    ;;
esac

case "$(uname -m)" in
  arm64 | aarch64) arch="arm64" ;;
  x86_64 | amd64) arch="x64" ;;
  *)
    say "unsupported CPU: $(uname -m)"
    exit 1
    ;;
esac

artifact="tally-$os-$arch"
url="$base_url/$artifact"
tmp="${TMPDIR:-/tmp}/$artifact.$$"

mkdir -p "$install_dir"

if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$url" -o "$tmp"
elif command -v wget >/dev/null 2>&1; then
  wget -qO "$tmp" "$url"
else
  say "missing required command: curl or wget"
  exit 1
fi

chmod 755 "$tmp"
mv "$tmp" "$bin"

say "installed: $bin"
case ":$PATH:" in
  *":$install_dir:"*) ;;
  *) say "add this to PATH: $install_dir" ;;
esac
say "try: tally ."
