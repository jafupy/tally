# Tally
A line counter for codebases.

Tally counts files, lines, blanks, comments, and code, ordered by lines of code.

```text
$ tally .
Language Files Lines Blank Comment  Code
-------- ----- ----- ----- ------- -----
Rust         6 1,235   184       4 1,047
TOML         9   972   146       2   824
Text         1   200    73       0   127
-------- ----- ----- ----- ------- -----
Total       16 2,407   403       6 1,998
```

Use `tally --json .` to emit the same results as structured JSON.
Use `tally --debug .` to print scan timing, CPU usage, worker and queue activity,
and unknown file formats to stderr. It can be combined with `--json`.

For a detailed timestamped JSONL trace, build with `cargo build --release --features trace`
and run `tally --debug=max .`. This appends to `.tallydebug` in the current
directory. It records per-file and per-line activity, so traces of large trees
can be very large. `clock_count` is a monotonic clock tick count (not CPU cycles).
Tally excludes this trace file from directory scans and rejects it as an explicit input.

Limit a scan with repeatable glob filters, or ask git for the relevant files:

```sh
tally --include '*.rs' --exclude 'target/**' .
tally --tracked .
tally . --diff          # compare the working tree against HEAD
tally src --diff main   # compare src against main
tally --diff            # default path . and revision HEAD
tally --diff HEAD~1     # default path . with an explicit revision
```

Diff syntax is `tally [path] --diff [revision]`. Put an explicit path before
`--diff`; the revision is optional and defaults to `HEAD`. Without `--diff`,
Tally counts lines as usual.

Use `tally -` to count text read from standard input.

## Install

```sh
curl -fsSL https://jafupy.com/tally.sh | sh
tally .
```

Installs the binary to `~/.local/bin` and the manual to
`~/.local/share/man/man1/tally.1`. Run `man tally` for usage, examples, Git
behaviour, and exit codes. If your system cannot find the manual, add this to
your shell configuration (the trailing colon keeps the system manual paths):

```sh
export MANPATH="$HOME/.local/share/man:${MANPATH:-}"
```

The installer supports macOS ARM64 and Linux x86_64. Set `TALLY_INSTALL_DIR`
to change the binary directory and `TALLY_MAN_DIR` to change the manual root
(the installer adds `man1`). Set `TALLY_VERSION` to a release tag to pin a
release. These variables must be passed to `sh`, for example:

```sh
curl -fsSL https://jafupy.com/tally.sh | TALLY_VERSION=v1.3 sh
```

Older releases have no manual asset: the binary still installs, with a warning.
A failed manual download or installation also warns without failing the binary
installation. An existing manual is left untouched if the download fails;
it may describe a different version after a downgrade.

Run `tally --version` to print the installed version and check GitHub for a newer
release. When one is available, Tally shows its release notes and offers to
download and install the matching release binary.
This built-in updater does not update the manual; rerun the installer to refresh
both. It does not change your shell configuration.

### Install from source

With Rust and Cargo installed, run from the repository root:

```sh
cargo build --release --locked
mkdir -p "$HOME/.local/bin" "$HOME/.local/share/man/man1"
install -m 755 target/release/tally "$HOME/.local/bin/tally"
install -m 644 man/tally.1 "$HOME/.local/share/man/man1/tally.1"
```

Ensure `~/.local/bin` is on `PATH`; use the `MANPATH` setting above if needed.
`cargo install --path .` installs only the binary, so copy the manual separately.
Native release downloads also include a standalone `tally.1` asset for manual
installation without a Rust toolchain.

In builds with the v1.4 scan and debug features, `tally -d` (or
`tally --debug=summary`) prints timing, CPU, and unknown-format diagnostics to
standard error, plus worker and queue measurements for directory scans.
`tally --debug=max` also appends an event trace
to `.tallydebug` in the current directory; it requires a source build with
`cargo build --release --features trace`. Regular release binaries do not
include that feature. The manual also covers the v1.4 Git filters, tracked-file
and diff modes; use a build that includes those v1.4 features.

GPL-3.0-or-later. Counts faster than anyone asked it to.
