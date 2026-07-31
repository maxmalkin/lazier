# lazier

A terminal user interface for git. It is written in Rust with
[ratatui](https://ratatui.rs) and [gitoxide](https://github.com/GitoxideLabs/gitoxide).

lazier does the same daily work as [lazygit](https://github.com/jesseduffield/lazygit),
but it uses less memory and less processor time on a large repository.

## Why it is fast

lazygit starts a `git` process for each read, then reads the text output.
lazier reads the repository in its own process with gitoxide. There is no
process to start and no text to parse.

Three more rules keep the program quick:

- The user interface thread does no git work. Worker threads do all reads.
  They send the results through one channel.
- The main loop waits on that channel. The program uses no processor time
  when you do not touch a key.
- A list shows only the rows in view. A repository with one million commits
  uses the same memory as a small one.

Writes are different. A write goes to the `git` command. Your hooks, your
credential helper, and your GPG key continue to work.

## Speed and memory

Apple Silicon, macOS, git 2.50.1. lazygit 0.61.1, gitui 0.28.1, lazier
0.1.0. Each program
starts in a real terminal of 80 by 24, draws its first screen, then quits.
Less is better in each column.

**A repository with 10 000 changed files**

| Program | Processor time | Memory |
|---------|---------------:|-------:|
| lazygit | 721 ms | 49 MB |
| gitui | 150 ms | 25 MB |
| **lazier** | **163 ms** | **16 MB** |

**The Linux kernel repository, 1.3 million commits**

| Program | Start | Scroll 300 commits | Memory |
|---------|------:|-------------------:|-------:|
| lazygit | 1963 ms | 2556 ms | 135 MB |
| gitui | 732 ms | 939 ms | 303 MB |
| **lazier** | **1247 ms** | **1232 ms** | **79 MB** |

**Processor time when you touch no key**

| Program | Idle |
|---------|-----:|
| lazygit | 0.0 % |
| gitui | 0.9 % |
| **lazier** | **0.0 %** |

Program file size: lazier 3.4 MB, gitui 9.5 MB, lazygit 17 MB.

What the numbers say:

- lazier uses the least memory in each test. On the kernel repository it uses
  1.7 times less than lazygit and 3.8 times less than gitui.
- lazier uses 4.4 times less processor time than lazygit on 10 000 changed
  files, and about 2 times less on a scroll through the kernel history.
- gitui is quicker than lazier to start on the kernel repository, but it
  needs 3.8 times more memory. It reads the whole object database at the
  start. lazier reads only the parts it must show.

Note on the clock: all three programs show their first screen in well under
one second, thus wall-clock times are almost equal. Processor time and memory
are the numbers that separate them.

To make these numbers again on your machine:

```sh
bench/setup.sh linux          # this downloads about 1.5 GB
LAZIER=lazier bench/run.sh
```

## Install

### From the source

You need Rust 1.85 or later.

```sh
cargo install --git https://github.com/maxmalkin/lazier
```

Or clone the repository first:

```sh
git clone https://github.com/maxmalkin/lazier
cd lazier
cargo install --path .
```

### From a release

Get the archive for your platform from the
[releases page](https://github.com/maxmalkin/lazier/releases). Then put the
program on your path:

```sh
tar -xzf lazier-<target>.tar.gz
sudo mv lazier /usr/local/bin/
```

## Use

Start the program in a git repository:

```sh
lazier
```

Press `?` for the full list of keys. The bar at the bottom always shows the
keys for the panel in focus.

### Panels

| Key | Panel | What it shows |
|-----|-------|---------------|
| `1` | Status | The branch, and how far it is from the upstream branch |
| `2` | Files | The changed files as a tree |
| `3` | Branches | The branches, the newest first, with `↑` and `↓` counts |
| `4` | Commits | The commit graph |
| `5` | Stash | The stash entries |
| `0` | Diff | The diff of the selected file or commit |
| `@` | Command log | Each git command, its result, and its time |

### Keys

Move with `j` and `k`. Move one page with `ctrl-d` and `ctrl-u`. Go to the
top or the end with `g` and `G`. Change the panel with `tab` or with the
number keys.

**Files.** Press `space` to stage or unstage a file or a directory. Press `a`
to stage all files. Press `enter` on a file to stage its hunks one at a time.
Press `enter` on a directory to fold it. Press `c` to open the commit window.
Press `C` to write the message in your editor. Press `s` to make a stash.
Press `o` or `t` to take ours or theirs in a conflict.

**Branches.** Press `enter` to check out. Press `n` for a new branch. Press
`d` to delete, or `D` to delete by force. Press `R` for a new name. Press `m`
to merge the branch into the current one. Press `P`, `p`, or `f` to push,
pull, or fetch.

**Commits.** Press `enter` for the full graph view. Press `i` to start an
interactive rebase. Press `w` to give the commit a new message. Press `v` to
revert it. An `↑` marks a commit that the upstream branch does not have.

**Interactive rebase.** Set an action on each commit with `p`, `r`, `e`, `s`,
`f`, or `d`. Move a commit with `J` and `K`. Press `enter` to run the rebase.
If the rebase stops, press `c` to continue, `s` to skip, or `A` to abort.

## What is not there yet

- Staging one line, not the full hunk
- Custom patches from an old commit
- Bisect and worktrees
- A configuration file

## License

MIT. See [LICENSE](LICENSE).
