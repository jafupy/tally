# tally

Fast, focused codebase accounting in Rust.

Tally walks a codebase, respects its Git ignore rules, detects languages from filenames, extensions, and common shebangs, then reports files, total lines, blanks, comments, and code. It is deliberately small, parallel when useful, and mildly obsessive about avoiding unnecessary work.

## Install

Prebuilt releases are available for macOS on Apple Silicon and Linux x86_64. Other platforms can [build from source](#development).

On macOS or Linux, this installs the latest release to `~/.local/bin`:

```sh
curl -fsSL https://jafupy.com/tally.sh | sh
```

The script detects the platform before downloading. Intel Macs, ARM Linux machines, and Windows are asked to build from source.

Alternatively, build it yourself:

```sh
git clone https://github.com/jafupy/tally.git
cd tally
cargo build --release
./target/release/tally .
```

## Use

```sh
tally                 # count the current directory
tally path/to/project # count a directory or one file
tally --all .         # include files ignored by Git
tally -j 8 .          # use eight scan workers
tally -v .            # report unrecognised file formats
```

By default, directory scans use Git's ignore rules and select up to four workers. Small directories are scanned serially; a single file always uses one worker. Progress is shown only when stderr is an interactive terminal.

Example output:

```text
Language Files Lines Blank Comment Code
-------- ----- ----- ----- ------- ----
Rust         6  1310   195       4 1111
TOML         9   974   146       2  826
-------- ----- ----- ----- ------- ----
Total       15  2284   341       6 1937
```

## Counting rules

Tally counts a non-empty line as a comment only when its first non-whitespace bytes begin with a configured line- or block-comment marker. Everything else is code. This makes its results predictable and fast, but it is not a full parser and does not try to identify inline comments.

Binary and non-UTF-8 files are skipped. Unknown text formats are grouped under `Unknown`; use `--verbose` to see the extensions or filenames behind that group.

Language definitions and extension disambiguation rules live in [`data/languages`](data/languages) and [`data/files.toml`](data/files.toml).

## Development

```sh
cargo test
cargo run -- .
```

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
