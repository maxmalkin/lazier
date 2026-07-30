# Benchmarks

Machine: macOS (Darwin 25.5.0), Apple Silicon. 2026-07-30.
Method: `bench/run.sh` — each TUI driven in an 80x24 pty by `bench/tui.py`.
All wall times include a constant **0.7s key-send delay**; compare columns to
each other, not to zero. CPU (user+sys) is the honest startup-work signal.

## Baselines (phase 0)

### startup+quit — dirty10k (10k dirty files)

| tool | wall (mean) | CPU (user+sys) | peak RSS |
|---|---|---|---|
| lazygit | 810 ms | 743 ms | 52 MB |
| gitui | 806 ms | 203 ms | 28 MB |
| lazier | — | — | — |

### startup+quit — bigdiff (5k-line modified file)

| tool | wall (mean) | CPU (user+sys) | peak RSS |
|---|---|---|---|
| lazygit | 806 ms | 512 ms | 28 MB |
| gitui | 819 ms | 88 ms | 19 MB |
| lazier | — | — | — |

### idle CPU % (5s sample, dirty10k)

| tool | idle CPU |
|---|---|
| lazygit | 0.0 |
| gitui | 0.8 |
| lazier | — |

### linux.git (1.3M commits, blobless clone)

| metric | lazygit | gitui | lazier |
|---|---|---|---|
| startup+quit wall | 763 ms | 788 ms | |
| startup+quit CPU (user+sys) | 460 ms | **1715 ms** | |
| scroll 300 commits wall | 768 ms | 851 ms | |
| peak RSS | 68 MB | **230 MB** | |

gitui pays 3.7x the CPU and 3.4x the RAM of lazygit here — libgit2 eagerly
loading the odb. This is the gap gix's lazy mmap'd packfile access must beat:
target <68 MB RSS and <460 ms CPU on startup.

## Phase 2 gate (from the plan)

- startup on linux.git ≤ gitui, « lazygit
- peak RSS ≥3x better than lazygit on dirty10k (target: ≤17 MB)
- 300-commit scroll on linux.git with no stall

## Reproduce

```sh
bench/setup.sh          # dirty10k + bigdiff
bench/setup.sh linux    # + linux blobless clone (~1.5GB)
bench/run.sh            # baselines
LAZIER="lazier" bench/run.sh   # include our binary, once it exists
```
