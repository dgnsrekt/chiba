#!/bin/sh
# How far has this fork drifted from webstonehq/tuxedo?
#
# Upstream's bug fixes are chiba's bug fixes, and the cost of merging grows
# with the gap. Run this every week or two; merging one commit is trivial,
# merging six months of them is a project.
set -eu

if ! git remote get-url upstream >/dev/null 2>&1; then
    echo "no 'upstream' remote — add it with:"
    echo "  git remote add upstream https://github.com/webstonehq/tuxedo.git"
    exit 1
fi

git fetch --quiet upstream
behind=$(git rev-list --count HEAD..upstream/main)
ahead=$(git rev-list --count upstream/main..HEAD)

echo "chiba is $ahead commit(s) ahead of upstream/main, $behind behind."

if [ "$behind" -eq 0 ]; then
    echo "Nothing to merge."
    exit 0
fi

echo
echo "New upstream commits:"
git log --oneline HEAD..upstream/main

echo
echo "Of those, the ones touching files chiba rewrote — this is where the"
echo "merge conflicts will be:"
git diff --name-only HEAD...upstream/main -- src/ \
    | grep -E 'todo\.rs|core/|cmd/|ui/logo\.rs|sample\.rs|cli\.rs' \
    | sed 's/^/  /' \
    || echo "  none — should merge cleanly"

echo
echo "To merge and verify:  mise run upstream_merge"
echo "                 or:  git merge upstream/main && cargo test"
