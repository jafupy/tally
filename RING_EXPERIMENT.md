# Lock-free MPMC ring experiment

Current worktree status: an own `std::fs::read_dir` crawler with two bounded
lock-free MPMC rings. Directory jobs carry inherited ignore-rule state; the
`ignore` crate is used only as a pattern matcher. Every worker can list
directories and count file batches. Worker count starts at one, grows quickly
under queue pressure, and never shrinks during a scan. The default file batch
size is 32. This is still an isolated experiment, not part of the PR.

## Shared-worker crawler, final comparison (10 runs each per order, `~/Developer`)

Full JSON output matched the PR baseline exactly in both modes: 57,111 files
normally, 94,276 files with `-a`.

| Mode and order | PR baseline | Shared crawler |
| --- | ---: | ---: |
| `-a`, baseline first | 1.288 s | 1.193 s |
| `-a`, crawler first | 1.287 s | 1.188 s |
| Normal, baseline first | 762 ms | 688 ms |
| Normal, crawler first | 781 ms | 689 ms |

Wall time improved about 8% with `-a` and 10–12% normally. System CPU rose
from roughly 3.4 s to 18 s with `-a`, and 1.8 s to 10.2 s normally. The
tradeoff is worthwhile only if wall time takes priority over machine load.

File batch tuning (10 runs each): batch 16/32 improved `-a` by about 2% over
per-file queueing; normal-mode batches 16–128 were close. Default 32 is a
compromise. Directory chunk depths 2 and 3 preserved counts but were about
1% slower with `-a` and tied normally, so directory chunking was removed.

## Drain files before each directory (10 runs each per order)

Tried `while file batch available { count batch }`, then one directory,
instead of counting one batch then one directory. Full JSON output matched.

| Mode and order | Alternating | Drain-first |
| --- | ---: | ---: |
| `-a`, alternating first | 1.166 s | 1.174 s |
| `-a`, drain first | 1.169 s | 1.166 s |
| Normal, alternating first | 671 ms | 675 ms |
| Normal, drain first | 677 ms | 672 ms |

No repeatable speedup. Draining also risks postponing directory discovery while
other workers continue to publish files, so the alternating loop was kept.

## Two-pool pipeline (50 runs each per order, `~/Developer`)

Full JSON output again matched baseline exactly in both modes.

| Mode and order | Baseline | Pipeline |
| --- | ---: | ---: |
| `-a`, baseline first | 1.295 s | 1.407 s |
| `-a`, pipeline first | 1.345 s | 1.402 s |
| Normal, baseline first | 783 ms | 801 ms |
| Normal, pipeline first | 814 ms | 858 ms |

The pipeline loses in both modes, including against the preceding shared-worker
crawler. It spends less total CPU than that crawler on `-a` but takes longer,
suggesting the fixed split between directory and counting workers leaves useful
capacity idle when the mix of work changes. That is a hypothesis, not an
isolated measurement. This variant should not be merged.

## Own crawler (50 runs each per order, `~/Developer`)

Full JSON output matched baseline exactly in both modes: 57,111 files normally,
94,276 files with `-a`.

| Mode and order | Baseline | Own crawler |
| --- | ---: | ---: |
| `-a`, baseline first | 1.282 s | 1.194 s |
| `-a`, crawler first | 1.390 s | 1.212 s |
| Normal, baseline first | 766 ms | 823 ms |
| Normal, crawler first | 805 ms | 790 ms |

The second normal-mode pass was noisy (crawler range 686–874 ms). The own
crawler consistently improves `-a` wall time but is not a reliable win in
normal mode. It uses much more system CPU: roughly 18 s vs 3.4 s per `-a`
run, and 11.8 s vs 1.8 s per normal run in the first order. The likely next
experiment is to process a small directory chunk per job rather than queue
every directory; this would retain inherited rule state while reducing ring
traffic. Do not benchmark further without coordinating with the user.

## Previous depth-3 walker prototype

Question: can workers publish short directory walks and file batches to a
bounded `ArrayQueue` so idle workers help with busy subtrees?

The prototype uses depth-3 walk jobs, batches of 128 files, and an 8,192-job
ring. It starts one worker and adds workers as queued work appears, up to the
machine's available parallelism. Workers remain until the scan finishes.
If the ring fills, a producer keeps excess jobs locally instead of blocking.

On `~/Developer`, default mode counted exactly 57,111 files and 10,940,608
lines in both implementations. `-a` counted exactly 94,276 files and
15,227,266 lines in both.

| Scan | Baseline | Ring | Runs |
| --- | ---: | ---: | ---: |
| Default, baseline first | 761 ms | 724 ms | 50 each |
| Default, ring first | 817 ms | 734 ms | 50 each |
| `-a`, baseline first | 1.438 s | 1.528 s | 50 each |
| `-a`, ring first | 1.303 s | 1.604 s | 50 each |

These 50-run passes were started after the user cleared the machine. The
baseline still drifted between passes. The ring consistently improved default
wall time, but it consistently slowed `-a`. Its CPU time was much higher:
about 11.6 seconds vs 2.4 seconds for default in the first pass, and about
24.3 seconds vs 4.9 seconds for `-a`. It should not replace the current
walker as-is.

Tuning pilots: depth 1 was slow (~830 ms with 18 workers); depth 2 reached
~757 ms; depth 3 reached ~713-722 ms; depths 4-5 were variable and generally
slower. File batches of 32, 128, and 512 were nearly tied at depth 3.
