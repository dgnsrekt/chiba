#!/usr/bin/env python3
"""Find every line where the fork rename turned 'tuxedo' into 'chiba'.

A blind search-and-replace is fine for identifiers and paths, but it also
rewrites the prose that *distinguishes* the fork from upstream — sentences
about what tuxedo does, what it lacked, or what chiba inherited. Those become
false claims. This lists them all for review.
"""
import re
import subprocess
import sys

files = subprocess.run(
    ["git", "ls-tree", "-r", "--name-only", "HEAD", "src/"],
    capture_output=True, text=True, check=True,
).stdout.split()

hits = []
for f in files:
    if not f.endswith(".rs"):
        continue
    up = subprocess.run(["git", "show", f"upstream/main:{f}"],
                        capture_output=True, text=True)
    if up.returncode:
        continue  # file is new in the fork
    old_lines = up.stdout.splitlines()
    new_lines = open(f, encoding="utf8").read().splitlines()
    old_set = {}
    for line in old_lines:
        if "tuxedo" in line.lower():
            key = re.sub(r"tuxedo", "chiba", line, flags=re.I)
            old_set.setdefault(key.strip(), line.strip())
    for n, line in enumerate(new_lines, 1):
        s = line.strip()
        if "chiba" in s.lower() and s in old_set:
            hits.append((f, n, old_set[s], s))

print(f"{len(hits)} renamed line(s)\n")
for f, n, before, after in hits:
    # Prose about upstream is the risky kind; identifiers and paths are fine.
    prose = bool(re.match(r"^\s*(//|///|//!)", after)) or '"' in after
    flag = "REVIEW" if prose else "      "
    print(f"{flag} {f}:{n}")
    print(f"        was: {before}")
    print(f"        now: {after}")
