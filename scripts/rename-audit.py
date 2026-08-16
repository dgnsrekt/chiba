#!/usr/bin/env python3
"""Find every line where the fork rename turned 'tuxedo' into 'chiba'.

A blind search-and-replace is fine for identifiers and paths, but it also
rewrites the prose that *distinguishes* the fork from upstream — sentences
about what tuxedo does, what it lacked, or what chiba inherited. Those become
false claims that compile, pass every test, and read as authoritative.

Usage:
    scripts/rename-audit.py              audit everything
    scripts/rename-audit.py --since REF  only files upstream touched since REF

`--since` is what makes this usable after a merge: the full list is ~100 lines
of already-reviewed identifiers, and the only ones that matter are the prose
lines that arrived with the merge.
"""
import re
import subprocess
import sys


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, text=True)


def main():
    since = None
    if "--since" in sys.argv:
        since = sys.argv[sys.argv.index("--since") + 1]

    files = git("ls-tree", "-r", "--name-only", "HEAD", "src/").stdout.split()
    files = [f for f in files if f.endswith(".rs")]

    if since:
        # Three dots, not two. On a fork, `since..upstream/main` lists every
        # file where the two differ — which is all of them — so the report
        # drowned in ~100 already-reviewed lines on the first real merge.
        # `since...upstream/main` is "what upstream changed since the merge
        # base", which is the only thing a post-merge audit cares about.
        touched = set(git("diff", "--name-only", f"{since}...upstream/main").stdout.split())
        files = [f for f in files if f in touched]
        if not files:
            print(f"No src/ files changed upstream since {since} — nothing to audit.")
            return 0

    hits = []
    for f in files:
        up = git("show", f"upstream/main:{f}")
        if up.returncode:
            continue  # new in the fork; nothing upstream to compare against
        renamed = {}
        for line in up.stdout.splitlines():
            if "tuxedo" in line.lower():
                key = re.sub(r"tuxedo", "chiba", line, flags=re.I).strip()
                renamed.setdefault(key, line.strip())
        for n, line in enumerate(open(f, encoding="utf8").read().splitlines(), 1):
            s = line.strip()
            if "chiba" in s.lower() and s in renamed:
                hits.append((f, n, renamed[s], s))

    # Prose is the risky kind — identifiers, paths and format strings that name
    # the binary are supposed to change.
    def is_prose(line):
        return bool(re.match(r"^\s*(//|///|//!)", line))

    prose = [h for h in hits if is_prose(h[3])]
    scope = f" (since {since})" if since else ""
    print(f"{len(hits)} renamed line(s){scope}; {len(prose)} are prose and need eyes\n")

    for f, n, before, after in prose:
        print(f"REVIEW {f}:{n}")
        print(f"        was: {before}")
        print(f"        now: {after}")

    if prose:
        print("\nCheck each: does the sentence describe *this* binary (fine) or")
        print("something upstream did, lacked, or originated (now a false claim)?")
    return 0


if __name__ == "__main__":
    sys.exit(main())
