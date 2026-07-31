#!/bin/bash
# Build benchmark corpora. Usage: ./setup.sh [linux]
#   no args -> dirty10k + bigdiff (fast, local)
#   linux   -> also blobless-clone linux.git (~1.5GB download, slow)
set -euo pipefail
cd "$(dirname "$0")/corpora"

# dirty10k: 10k dirty worktree files (status stressor)
if [ ! -d dirty10k ]; then
  mkdir dirty10k && cd dirty10k && git init -q
  for i in $(seq 1 10000); do echo "line $i" > "f$i"; done
  git add -A && git commit -qm init
  for i in $(seq 1 10000); do echo "$RANDOM" >> "f$i"; done
  cd ..
  echo "dirty10k done"
fi

# bigdiff: 5k-line file, fully modified (diff stressor)
if [ ! -d bigdiff ]; then
  mkdir bigdiff && cd bigdiff && git init -q
  seq 1 5000 | sed 's/^/original line /' > big.txt
  git add -A && git commit -qm init
  seq 1 5000 | sed 's/^/changed line /' > big.txt
  cd ..
  echo "bigdiff done"
fi

# linux: 1.3M commits (log stressor). Blobless partial clone: full history,
# no file contents. The log traversal only needs commits and trees.
if [ "${1:-}" = "linux" ] && [ ! -d linux ]; then
  git clone --filter=blob:none https://github.com/torvalds/linux.git linux
  echo "linux done"
fi
