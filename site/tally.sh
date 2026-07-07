#!/usr/bin/env sh
set -eu

repo="${TALLY_REPO:-https://github.com/jafupy/tally}"
install_dir="${TALLY_INSTALL_DIR:-$HOME/.local/bin}"
bin="$install_dir/tally"
tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/tally.XXXXXX")"
src="$tmpdir/source"
installed_rust=0

say() {
  printf '%s\n' "$*"
}

cleanup() {
  rm -rf "$tmpdir"
}

trap cleanup EXIT INT TERM

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    say "missing required command: $1"
    exit 1
  fi
}

need git

if ! command -v cargo >/dev/null 2>&1; then
  if command -v curl >/dev/null 2>&1; then
    fetch_rustup='curl -fsSL https://sh.rustup.rs'
  elif command -v wget >/dev/null 2>&1; then
    fetch_rustup='wget -qO- https://sh.rustup.rs'
  else
    say "missing required command: cargo, or curl/wget to install temporary Rust"
    exit 1
  fi

  installed_rust=1
  export CARGO_HOME="$tmpdir/cargo"
  export RUSTUP_HOME="$tmpdir/rustup"
  export PATH="$CARGO_HOME/bin:$PATH"

  say "installing temporary Rust toolchain"
  sh -c "$fetch_rustup" | sh -s -- -y --no-modify-path --profile minimal
fi

say "cloning $repo"
git clone --depth 1 "$repo" "$src"

say "building tally"
(cd "$src" && cargo build --release --locked)

mkdir -p "$install_dir"
cp "$src/target/release/tally" "$bin"
chmod 755 "$bin"

if [ "$installed_rust" -eq 1 ]; then
  say "removed temporary Rust toolchain"
fi

say "installed: $bin"
case ":$PATH:" in
  *":$install_dir:"*) ;;
  *) say "add this to PATH: $install_dir" ;;
esac
say "try: tally ."
