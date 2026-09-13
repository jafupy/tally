#!/usr/bin/env sh
set -eu

repo="${TALLY_REPO:-jafupy/tally}"
version="${TALLY_VERSION:-latest}"
install_dir="${TALLY_INSTALL_DIR:-$HOME/.local/bin}"
man_dir="${TALLY_MAN_DIR:-$HOME/.local/share/man}"
executable="tally"
tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/tally.XXXXXX")"

say() {
  printf '%s\n' "$*"
}

cleanup() {
  rm -rf "$tmpdir"
}

trap cleanup EXIT INT TERM

unsupported() {
  say "no prebuilt Tally release is available for $1"
  say "supported platforms: macOS ARM64 and Linux x86_64"
  say "build from source instead: https://github.com/$repo#development"
  exit 1
}

case "$(uname -s)" in
  Darwin)
    case "$(uname -m)" in
      arm64 | aarch64)
        target="aarch64-apple-darwin"
        asset="tally-mac-arm"
        ;;
      *) unsupported "macOS $(uname -m)" ;;
    esac
    ;;
  Linux)
    case "$(uname -m)" in
      x86_64 | amd64)
        target="x86_64-unknown-linux-gnu"
        asset="tally-linux-x86_64"
        ;;
      *) unsupported "Linux $(uname -m)" ;;
    esac
    ;;
  *) unsupported "$(uname -s) $(uname -m)" ;;
esac

if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL --connect-timeout 10 --max-time 60 "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -qO "$2" --timeout=10 --tries=1 "$1"; }
else
  say "missing required command: curl or wget"
  exit 1
fi

if [ "$version" = "latest" ]; then
  release_url="https://github.com/$repo/releases/latest/download"
else
  release_url="https://github.com/$repo/releases/download/$version"
fi

say "downloading Tally $version for $target"
fetch "$release_url/$asset" "$tmpdir/$executable"

mkdir -p "$install_dir"
bin="$install_dir/$executable"
install -m 755 "$tmpdir/$executable" "$bin"

say "installed: $bin"
if fetch "$release_url/tally.1" "$tmpdir/tally.1"; then
  if mkdir -p "$man_dir/man1" && install -m 644 "$tmpdir/tally.1" "$man_dir/man1/tally.1"; then
    say "installed: $man_dir/man1/tally.1"
    say "manual: man tally"
    say "if not found, add this directory to MANPATH (keep a trailing colon): $man_dir"
  else
    say "warning: could not install the manual; Tally is installed" >&2
  fi
else
  say "warning: manual unavailable for $version (older releases have none); Tally is installed" >&2
fi
case ":$PATH:" in
  *":$install_dir:"*) ;;
  *) say "add this to PATH: $install_dir" ;;
esac
say "try: tally ."
