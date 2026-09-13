# Optimisation Attempts

This file records performance experiments that were tried for `tally` and should not be repeated without new evidence.

## Kept

### Adaptive small-directory scan

Status: kept.

For default directory scans, `tally` now probes up to 128 files before starting the parallel walker. Small directories are scanned serially from the collected file list, avoiding worker startup overhead.

Result:

- Tiny copied repo improved from roughly `5.2ms` to `2.5ms`.
- Full `~/benchmarks/tally-corpus` output matched.
- No meaningful full-corpus regression.

## Rejected Or Inconclusive

### Rayon for parsing

Experiment:

- Keep `ignore` for discovery.
- Collect file paths.
- Parse with Rayon over chunks.

Result:

- Output matched.
- Full corpus was slightly slower: about `2.076s` vs `2.014s`.
- `-j 8` looked worse.

Reason rejected:

The current large-tree path already parses inside `ignore` parallel walker workers. Rayon added a two-phase `Vec<PathBuf>` collection step and lost traversal/parse overlap.

### Rayon over the walker

Experiment:

- Replace the large-tree walker path with an `ignore` walker bridged into Rayon.

Result:

- Output matched.
- No reliable win; noisy tie or loss.
- Higher thread counts dragged.

Reason rejected:

Extra scheduling did not beat `ignore`'s own parallel walker.

### Crawl/parse queue pipeline

Experiment:

- Split crawling and parsing with bounded queues.
- Tried a simple serial crawler feeding parser workers.
- Then tried a more honest parallel-crawler-to-parser-worker split.
- Also tried an oversubscribed version with crawler workers plus parser workers.

Result:

- Output matched.
- Serial crawler version was clearly slower: about `2.290s` vs `1.913s`.
- Strict split under default worker budget was much slower, about `4.01s` in a single run.
- Oversubscribed version sometimes completed quickly but showed pathological long CPU-heavy tails under repeated `hyperfine` runs.

Reason rejected:

The current walker already overlaps traversal and parsing fairly well. The queue versions added channel handoff, path cloning, and bad tail behavior.

### Git discovery

Experiment:

- Use Git data such as `git ls-files` for discovery.

Result:

- Output matched in the experiment.
- No speedup.

Reason rejected:

It changes the product contract by relying on Git instead of `tally`'s own indexer/walker, and did not improve performance.

### Reusable parser buffers

Experiment:

- Reuse parser buffers per worker for extension disambiguation and comment parsing instead of allocating new temporary buffers.

Result:

- Output matched.
- Tied or slightly slower, around `2.52s` vs `2.49s`.

Reason rejected:

The added state and reuse did not reduce the measured cost.

### Buffer size changes

Experiments:

- Increase `BufReader` buffer to `256KiB`.
- Decrease buffer to `16KiB`.

Result:

- `256KiB`: noisy, no reliable win.
- `16KiB`: output matched but was slower, around `2.68s` vs `2.50s`.

Reason rejected:

The current `64KiB` buffer remains the best observed default.

### Plain/no-comment fast path

Experiment:

- Add a parser path for languages with no line or block comment markers, including `Unknown`, `Text`, and `Diff`.
- Count only blank/code lines.

Result:

- Output matched.
- Slower in the agent benchmark: baseline about `6.242s`, fast path about `6.841s`.

Reason rejected:

The specialized path did not beat the generic parser on the corpus.

### Whole-file read

Experiment:

- Read each file into a `Vec<u8>` and parse from memory.

Result:

- Too slow or stalled during testing.

Reason rejected:

Whole-file buffering increased memory traffic and did not fit the many-file corpus well.

### Hybrid mmap for large files

Experiment:

- Use `BufReader` for small files.
- Use `memmap2` for files above `1MiB` and `8MiB` thresholds.

Result:

- Output matched for both thresholds.
- `1MiB`: baseline about `6.267s`, mmap about `6.705s`.
- `8MiB`: baseline about `2.789s`, mmap about `5.481s`.

Reason rejected:

Mmap increased total time, and the conservative threshold still made the full corpus much slower.

### Long-line streaming parser

Experiment:

- Replace `partial_line: Vec<u8>` accumulation for lines spanning the reader buffer with an incremental long-line classifier/searcher.

Result:

- Output matched.
- Some noisy runs looked faster:
  - baseline `6.636s`, long-line `3.366s`
  - long-line `4.247s`, baseline `4.386s`
  - baseline `5.261s`, long-line `4.257s`
- A later serialized parent rerun was not usable because the baseline itself entered a pathological slow run.

Reason not merged:

The implementation added roughly 230 lines of parser complexity, and the speedup was not confirmed on a quiet remote run.

### Lowercase extension fast path

Experiment:

- Avoid repeated case-folding in extension lookup by lowercasing only when needed.

Result:

- Output matched.
- Clean-ish benchmark was a tie or slight loss: baseline about `2.647s`, fast path about `2.677s`.

Reason rejected:

Extension lookup is not hot enough for this change to matter.

### Generated extension match

Experiment:

- Generate a direct `match` over lowercase extensions, with uppercase fallback lowercasing once.

Result:

- Output matched.
- Serialized parent run was a tie: baseline `2.739s`, generated match `2.723s`, well inside noise.

Reason rejected:

No confirmed speedup, and it complicates generated lookup code.

### Byte-prefix UTF-8 avoidance

Experiment:

- Avoid UTF-8 conversion for the prefix path and operate more directly on bytes.

Result:

- Changed output by counting extra non-UTF-8 files.

Reason rejected:

Incorrect behavior.

### Smaller detection prefix

Experiment:

- Reduce `DETECTION_PREFIX_BYTES` from `16KiB` to `12KiB`, `8KiB`, `4KiB`, and `2KiB`.

Result:

- All smaller prefixes changed full-corpus output.
- Differences included skipped/non-UTF-8 files and language shifts across Rust, TypeScript, JavaScript, Markdown, C, Inno Setup, Text, and Unknown.

Reason rejected:

The 16KiB prefix is part of current semantics for binary/UTF-8 filtering and disambiguation on the corpus.

### SIMD UTF-8 validation

Experiment:

- Replace `std::str::from_utf8` in `text_prefix` with `simdutf8::basic::from_utf8`.

Result:

- Output matched.
- Remote corpus was noisy: one run looked faster, reversed order was a tie.
- Stable local `~/Projects` benchmark was slower: clean adaptive `450.3ms`, SIMD UTF-8 `465.3ms`.

Reason rejected:

No confirmed win, and it adds a dependency.

### ASCII prefix fast path

Experiment:

- Scan prefix once for NUL and ASCII.
- Return `from_utf8_unchecked` for all-ASCII prefixes.
- Fall back to the original full NUL check plus UTF-8 validation for non-ASCII prefixes.

Result:

- Output matched after fixing the non-ASCII fallback to keep the full NUL check.
- Local `~/Projects` benchmark was slower: clean adaptive `442.5ms`, ASCII fast path `461.2ms`.
- Remote benchmark hit a pathological long-tail run and was killed.

Reason rejected:

The manual byte loop was slower than the standard UTF-8 path on the stable local corpus.

### `memchr_iter`

Experiment:

- Use `memchr_iter` for line scanning.

Result:

- Neutral to slightly slower.

Reason rejected:

No measurable benefit over the current loop.

### Larger sink batch flush

Experiment:

- Change `FLUSH_EVERY_FILES` from `512` to `1024`, `2048`, and `4096`.

Result:

- All tested variants matched output.
- One noisy four-way run suggested `2048` might be best.
- A fresh serialized parent run with a clean `2048` binary was a tie:
  - baseline `2.747s`
  - `2048` `2.750s`

Reason rejected:

No confirmed speedup. Kept `512`.

### Fat LTO

Experiment:

- Use fatter link-time optimization settings.

Result:

- Runtime was neutral.
- Build time increased.

Reason rejected:

No runtime benefit.

### Native fd-relative crawler

Experiment:

- Replace the `ignore` walker for `--all` scans with a Unix `fdopendir`/`readdir` crawler.
- Open files relative to their parent directory with `openat` and parse already-open file descriptors.

Result:

- Matched `--all` output on the Zed corpus after reproducing hidden/VCS-directory filtering.
- Did not establish complete ignore-semantic parity on the full corpus.
- Was dramatically slower on local `~/Projects`: about `19.3s` versus `5.0s` for the generic walker.

Reason rejected:

`ignore` is already the deep module for parallel, Git-aware traversal. The prototype's shared directory queue, per-entry FFI loop, and incomplete filter implementation cost more than descriptor-relative opens saved.

### Fused prefix validation and line counting

Experiment:

- For files with unambiguous filename/shebang detection, validate NUL/UTF-8 and count the first 16 KiB in one pass.
- Continue normal line counting from byte 16 KiB; ambiguous formats retain the existing path.

Result:

- Unit tests passed and output matched on Zed and full local `~/Projects` with `-j4`.
- Local paired benchmark: baseline `426.6ms ± 11.4ms`; fused parser `446.9ms ± 4.1ms`.

Reason rejected:

The byte-by-byte UTF-8 state machine was slower than the current specialised UTF-8 validator plus `memchr` line scanner, despite eliminating the duplicate prefix pass.

### macOS worker QoS

Experiment:

- Elevate worker-thread QoS to prefer performance scheduling on macOS.

Result:

- Output matched and tests passed.
- No credible win; a paired run was slower (`1.315s ± 0.170` versus `0.796s ± 0.336`) and noisy.

Reason rejected:

The scheduler policy did not improve scan throughput reliably.

### `target-cpu=native`

Experiment:

- Build with `RUSTFLAGS='-C target-cpu=native'`.

Result:

- Output matched exactly.
- Clean local paired benchmark regressed: native `1.255s` mean (`1.154s` median) versus generic `1.043s` mean (`1.028s` median).

Reason rejected:

Platform-specific code generation was slower on the representative corpus.

### mimalloc

Experiment:

- Use `mimalloc` as the global allocator.

Result:

- Output matched on local `~/Projects` and the remote M2 corpus.
- Local results were order-sensitive and too noisy to trust.
- Serialized remote M2 runs rejected it in both orders:
  - baseline first: baseline `2.516s`, mimalloc `2.663s`;
  - mimalloc first: mimalloc `2.936s`, baseline `2.285s`.
- M2 peak memory also rose from about `9–10MiB` to `18.6MiB`.

Reason rejected:

It is slower and uses more memory on the benchmark target.

### Current stopping point

The credible one-shot, exact-scan micro-optimisation avenues have been exhausted. Keep the adaptive small-directory scan; do not revisit the rejected crawler, parser, scheduler, compiler-flag, or allocator experiments without materially new evidence. Future speed work requires either profile evidence for a new counting algorithm or an explicit semantic/product mode that deliberately does less work.

## Benchmark Notes

The remote M2 became noisy during later runs. In several cases, even the baseline binary entered pathological slow runs over 60 seconds while using high CPU. Treat any benchmark taken during overlapping sub-agent runs or during those pathological periods as contaminated.

For future optimisation work:

- Run only one remote benchmark at a time.
- Always compare output against `/tmp/tally-bench.eG8APF/adaptive-small-corpus.out`.
- Prefer fresh binaries built from the current integration tree over binaries produced in experiment worktrees.
- Require a serialized paired benchmark win before merging.

## Diagnostic Findings

### Sampling

Symbolized `sample` output on the M2 showed worker threads dominated by:

- `tally::file::parse_file` under `read`
- `tally::file::parse_file` under `open`
- smaller but visible `tally::file::count_lines`
- `ignore` walker work under `opendir`, `readdir`, `stat`, gitignore/glob matching

The main thread mostly waits for walker workers.

### Phase instrumentation

Temporary `TALLY_DIAG=1` instrumentation split parse work into open, prefix/detect, and line counting. On the full corpus, cumulative worker time was dominated by:

- Rust: many files; prefix/detect was much larger than line counting.
- Unknown: many files; prefix/detect was much larger than line counting.
- C: large line count; line counting was meaningful but still comparable to open/prefix work.
- JavaScript/C++/TypeScript/Markdown: lower than Rust/C/Unknown.

Long-line copying was small in aggregate on the M2 corpus, so the earlier long-line streaming result is unlikely to be a robust general win.

### Thread count

Diagnostic thread sweep:

- Local `~/Projects` was stable and clearly best at `-j4`.
- `-j5` and `-j6` got slower from system pressure.
- Remote corpus showed `-j1` and `-j2` were slower, `-j3` plausible, but the sweep became contaminated by a pathological `-j4` long-tail run.

Conclusion: keep the default cap at 4 workers.

### `--all` control

Experiment:

- Compare default ignore handling to `--all` on the full corpus.

Result:

- `--all` was only a tie/slight noisy improvement: about `2.649s` vs default `2.700s`.

Conclusion:

Gitignore matching is visible in profiles but not the dominant cost.
