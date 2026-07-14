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

## Install

```sh
curl -fsSL https://jafupy.com/tally.sh | sh
tally .
```

Installs to `~/.local/bin`.

GPL-3.0-or-later. Counts faster than anyone asked it to.
