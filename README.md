```  ,,                     ,,                 
`7MM                     db                 
  MM                                        
  MM   ,6"Yb.  M"""MMV `7MM  .gP"Ya `7Mb,od8
  MM  8)   MM  '  AMV    MM ,M'   Yb  MM' "'
  MM   ,pm9MM    AMV     MM 8M""""""  MM    
  MM  8M   MM   AMV  ,   MM YM.    ,  MM    
.JMML.`Moo9^Yo.AMMmmmM .JMML.`Mbmmd'.JMML.    
```

[![ci](https://github.com/maxmalkin/lazier/actions/workflows/ci.yml/badge.svg)](https://github.com/maxmalkin/lazier/actions/workflows/ci.yml)
[![release](https://img.shields.io/github/v/release/maxmalkin/lazier?color=green)](https://github.com/maxmalkin/lazier/releases)
[![msrv](https://img.shields.io/badge/msrv-1.85-blue)](https://www.rust-lang.org)
[![license](https://img.shields.io/github/license/maxmalkin/lazier?color=blue)](LICENSE)

A fast terminal user interface for git. Rust, [ratatui](https://ratatui.rs),
and [gitoxide](https://github.com/GitoxideLabs/gitoxide).

It does the work of [lazygit](https://github.com/jesseduffield/lazygit)
with less memory and less processor time, along with some new features.

## Speed and memory

Less is better.

| Test | | lazygit | lazier | |
|------|---|--------:|-------:|----:|
| 10 000 changed files | processor | 748 ms | **187 ms** | **~4x** |
| Refresh 12 times, 10 000 files | processor | 3477 ms | **178 ms** | **~20x** |
| Walk 300 files with diffs | processor | 1263 ms | **147 ms** | **~9x** |
| Linux kernel, 1.3M commits | scroll 2000 commits | 2196 ms | **1346 ms** | **~1.6x** |
| | peak RSS | 136 MB | **79 MB** | **~1.7x** |
| 5000-line diff | peak RSS | 27 MB | **10 MB** | **~2.7x** |
| Idle | processor | 0.3 % | **0.0 %** | — |
| Program file | | 17 MB | **3.4 MB** | **~5x** |

lazier uses less memory in every test and no processor time when you touch
no key. lazygit starts more quickly on the kernel repository: it runs
several `git` processes at once and pays for it in memory.

A file that changes outside the program needs no walk of the whole work
tree. lazier watches the work tree and looks only at what changed, which on
the kernel repository is **150 ms** in place of **1212 ms**.

**Why it is fast.** lazygit starts a `git` process for each read and parses
the text. lazier reads the repository in its own process with gitoxide.
Worker threads do all git work, thus the screen never waits. A list shows
only the rows in view. A refresh keeps the commits that are already in
memory when HEAD did not move, and looks at the whole work tree only when
a command could have changed a file. Writes still go to the `git` command,
thus your hooks, credential helper, and GPG key continue to work.

## Install

From the source, with Rust 1.85 or later:

```sh
cargo install --git https://github.com/maxmalkin/lazier
```

Or take an archive for your platform from the
[releases page](https://github.com/maxmalkin/lazier/releases). macOS and
Linux:

```sh
tar -xzf lazier-<target>.tar.gz && sudo mv lazier /usr/local/bin/
```

Windows: unzip `lazier-x86_64-pc-windows-msvc.zip`, then put `lazier.exe` in
a directory on your `PATH`.

There are programs for macOS, Linux, and Windows, each one for the x86-64
and the ARM processor.

## Use

Run `lazier` in a git repository. Press `?` for all keys. The bar at the
bottom shows the keys for the panel in focus.

| Key | Goes to |
|-----|---------|
| `1` `2` `3` `4` `5` | Status, Files, Branches, Commits, Stash |
| `0` | Diff |
| `W` | Worktrees |
| `@` | Command log: each command, its result, and its time |
