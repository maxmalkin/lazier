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

It does the daily work of [lazygit](https://github.com/jesseduffield/lazygit)
with less memory and less processor time.

## Speed and memory

Less is better.

| Test | | lazygit | gitui | lazier |
|------|---|--------:|------:|-------:|
| 10 000 changed files | processor | 721 ms | 150 ms | **163 ms** |
| | memory | 49 MB | 25 MB | **16 MB** |
| Linux kernel, 1.3M commits | start | 1963 ms | 732 ms | **1247 ms** |
| | scroll 300 | 2556 ms | 939 ms | **1232 ms** |
| | memory | 135 MB | 303 MB | **64 MB** |
| Idle | processor | 0.0 % | 0.9 % | **0.0 %** |
| Program file | | 17 MB | 9.5 MB | **3.4 MB** |

lazier uses the least memory in each test. gitui starts more quickly on the
kernel repository, but it needs 3.8 times more memory: it reads the whole
object database, and lazier reads only the parts it must show.

**Why it is fast.** lazygit starts a `git` process for each read and parses
the text. lazier reads the repository in its own process with gitoxide.
Worker threads do all git work, thus the screen never waits. A list shows
only the rows in view. Writes still go to the `git` command, thus your
hooks, credential helper, and GPG key continue to work.

## Install

From the source, with Rust 1.85 or later:

```sh
cargo install --git https://github.com/maxmalkin/lazier
```

Or take an archive for your platform from the
[releases page](https://github.com/maxmalkin/lazier/releases), then:

```sh
tar -xzf lazier-<target>.tar.gz && sudo mv lazier /usr/local/bin/
```

## Use

Run `lazier` in a git repository. Press `?` for all keys. The bar at the
bottom shows the keys for the panel in focus.

| Key | Panel |
|-----|-------|
| `1` `2` `3` `4` `5` | Status, Files, Branches, Commits, Stash |
| `0` | Diff |
| `@` | Command log, with the result and the time of each command |

Move with `j` and `k`, `ctrl-d` and `ctrl-u`, `g` and `G`.

- **Files.** `space` stage · `a` stage all · `enter` hunks, or fold a
  directory · `c` commit window · `C` your editor · `s` stash · `o` `t` take
  ours or theirs
- **Branches.** `enter` check out · `n` new · `d` `D` delete · `R` rename ·
  `m` merge · `P` `p` `f` push, pull, fetch
- **Commits.** `enter` graph view · `i` interactive rebase · `w` reword ·
  `v` revert · `↑` marks a commit the upstream branch does not have
- **Rebase.** `p` `r` `e` `s` `f` `d` set the action · `J` `K` move a commit
  · `enter` run. If it stops: `c` continue · `s` skip · `A` abort

## License

MIT. See [LICENSE](LICENSE).
