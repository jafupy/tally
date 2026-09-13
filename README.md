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

## Install

```sh
curl -fsSL https://jafupy.com/tally.sh | sh
tally .
```

Installs to `~/.local/bin`.

Run `tally --version` to print the installed version and check GitHub for a newer
release. When one is available, Tally shows its release notes and offers to
download and install the matching release binary.

GPL-3.0-or-later. Counts faster than anyone asked it to.
