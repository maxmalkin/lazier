#!/bin/bash
# Benchmark lazygit vs gitui (vs lazier via LAZIER=<cmd>) on the corpora.
# tui.py drives each TUI in a real pty; all timings include a constant 0.7s
# key-send delay, so compare numbers to each other, not to zero.
set -uo pipefail
cd "$(dirname "$0")/corpora"
TUI="python3 ../tui.py"

for repo in dirty10k bigdiff linux; do
  [ -d "$repo" ] || continue
  echo "=== startup+quit: $repo ==="
  hyperfine --warmup 1 -N --style basic \
    -n lazygit "$TUI q 120 lazygit -p $repo" \
    -n gitui   "$TUI q 120 gitui -d $repo" \
    ${LAZIER:+-n lazier "$TUI q 120 $LAZIER $repo"}

  echo "=== peak RSS: $repo ==="
  for cmd in "lazygit -p $repo" "gitui -d $repo" ${LAZIER:+"$LAZIER $repo"}; do
    rss=$($TUI q 120 $cmd 2>&1 >/dev/null | grep -o 'RSS_MB=[0-9]*')
    echo "  ${cmd%% *}: ${rss}"
  done
done

if [ -d linux ]; then
  echo "=== scroll 300 commits then quit: linux ==="
  J=$(printf 'j%.0s' $(seq 300))
  # lazygit: '2' focuses the commits panel first; gitui: '2' opens its log tab
  hyperfine --warmup 1 -N --style basic \
    -n lazygit "$TUI '2${J}q' 300 lazygit -p linux" \
    -n gitui   "$TUI '2${J}q' 300 gitui -d linux" \
    ${LAZIER:+-n lazier "$TUI '2${J}q' 300 $LAZIER linux"}
fi

echo "=== idle CPU % (5s sample, dirty10k) ==="
for cmd in "lazygit -p dirty10k" "gitui -d dirty10k" ${LAZIER:+"$LAZIER dirty10k"}; do
  $TUI '' 20 $cmd >/dev/null 2>&1 & driver=$!
  sleep 6
  app=$(pgrep -n -f "${cmd%% *}" 2>/dev/null)
  cpu=$([ -n "$app" ] && ps -o %cpu= -p "$app" || echo "?")
  echo "  ${cmd%% *}: ${cpu# }"
  kill $driver 2>/dev/null; [ -n "$app" ] && kill $app 2>/dev/null
done
wait 2>/dev/null
echo "done"
