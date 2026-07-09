# Tally
A line counter for codebases.

Tally counts files, lines, blanks, comments, and code, sorted by language.

```text
$ tally .
Language Files Lines Blank Comment Code
-------- ----- ----- ----- ------- ----
Rust         6  1235   184       4 1047
TOML         9   972   146       2  824
Text         1   200    73       0  127
-------- ----- ----- ----- ------- ----
Total       16  2407   403       6 1998
```

## Install

```sh
curl -fsSL https://jafupy.com/tally.sh | sh
tally .
```

Installs to `~/.local/bin`.

[Source on GitHub.](https://github.com/jafupy/tally)

GPL-3.0-or-later. Counts faster than anyone asked it to.
