```
$$\                     $$\
$$ |                    \__|
$$ | $$$$$$\  $$$$$$$$\ $$\  $$$$$$\   $$$$$$\
$$ | \____$$\ \____$$  |$$ |$$  __$$\ $$  __$$\
$$ | $$$$$$$ |  $$$$ _/ $$ |$$$$$$$$ |$$ |  \__|
$$ |$$  __$$ | $$  _/   $$ |$$   ____|$$ |
$$ |\$$$$$$$ |$$$$$$$$\ $$ |\$$$$$$$\ $$ |
\__| \_______|\________|\__| \_______|\__|
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
| 10 000 changed files | processor | 721 ms | **163 ms** | **~4.4x** |
| | peak RSS | 49 MB | **16 MB** | **~3x** |
| Linux kernel, 1.3M commits | start | 1963 ms | **1247 ms** | **~1.6x** |
| | scroll 300 commits | 2556 ms | **1232 ms** | **~2x** |
| | scroll 2000 commits | 2264 ms | **1316 ms** | **~1.7x** |
| | peak RSS | 135 MB | **64 MB** | **~2.1x** |
| Refresh 12 times, 10 000 files | processor | 4003 ms | **176 ms** | **~23x** |
| Walk 300 files with diffs | processor | 1106 ms | **169 ms** | **~6.5x** |
| Idle | processor | 0.2 % | **0.0 %** | — |
| Program file | | 17 MB | **3.4 MB** | **~5x** |

lazier uses less memory in every test, and it uses no processor time when
you touch no key.

**Why it is fast.** lazygit starts a `git` process for each read and parses
the text. lazier reads the repository in its own process with gitoxide.
Worker threads do all git work, thus the screen never waits. A list shows
only the rows in view. A refresh keeps the commits that are already in
memory when HEAD did not move. Writes still go to the `git` command, thus
your hooks, credential helper, and GPG key continue to work.

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
