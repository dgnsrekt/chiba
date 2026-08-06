#!/bin/sh
# Merge upstream/main, then prove the fork still holds.
#
# The rename audit runs here rather than being something to remember: a merge
# lands upstream's prose about tuxedo, and any of it that later gets renamed
# becomes a false claim that compiles and passes every test. The only moment
# it's cheap to catch is right now, while the merge is in front of you.
set -eu

before=$(git rev-parse HEAD)

git fetch --quiet upstream
if [ "$(git rev-list --count HEAD..upstream/main)" -eq 0 ]; then
    echo "Already up to date with upstream/main — nothing to merge."
    exit 0
fi

echo "Merging $(git rev-list --count HEAD..upstream/main) upstream commit(s)…"
git merge --no-edit upstream/main

echo
echo "── tests ─────────────────────────────────────────────────────────────"
cargo test --locked

echo
echo "── rename audit ──────────────────────────────────────────────────────"
python3 "$(dirname "$0")/rename-audit.py" --since "$before"

echo
echo "── still to check by hand ────────────────────────────────────────────"
echo "  * src/todo.rs conflicts — the markdown layer is the fork's core"
echo "  * new upstream strings naming todo.txt files that chiba stores as .md"
echo "  * mise run screenshots, if the UI moved"
