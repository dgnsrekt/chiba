#!/usr/bin/env python3
"""Render the chiba banner: the two marks side by side, same palette.

Left is tuxedo's bowtie (full-cell grid, 7x5), right is chiba's thrown rose
(half-block grid, 16x10). Both are lifted straight from the source so the
banner can't drift from what the terminal actually draws.
"""

import pathlib

BOWTIE = ["b     b", "bb k bb", "bbbkbbb", "bb k bb", "b     b"]

ROSE = [
    "..kkkk..........",
    ".kkkkkk.........",
    ".kkkkkk.........",
    "..kkkk.b........",
    "...kk...b.......",
    ".........b......",
    "......bbb.b.....",
    "...........b....",
    "............b...",
    ".............b..",
]

BG = "#1a1c1f"
ACCENT = "#8aa9c9"
RED = "#e07a7a"
DIM = "#454b54"
FG = "#c8ccd4"

W, H = 880, 280
PX = 11  # rose pixel size; bowtie cell is 2x this so both read at one weight


def rects(grid, ox, oy, pw, ph):
    """Emit one rect per filled cell. Empty is `.` in the rose grid and a
    space in the bowtie grid — both must be skipped."""
    out = []
    for r, row in enumerate(grid):
        for c, ch in enumerate(row):
            if ch in ". ":
                continue
            fill = RED if ch == "k" else ACCENT
            x, y = ox + c * pw, oy + r * ph
            out.append(f'<rect x="{x}" y="{y}" width="{pw}" height="{ph}" fill="{fill}"/>')
    return out


parts = [
    f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}">',
    f'<rect width="{W}" height="{H}" rx="14" fill="{BG}"/>',
]

# Bowtie: 7x5 cells, each cell 2 px wide (matches "██" per cell in the TUI).
bw, bh = 7 * PX * 2, 5 * PX * 2
parts += rects(BOWTIE, 220 - bw // 2, 80, PX * 2, PX * 2)

# Divider.
parts.append(
    f'<line x1="{W // 2}" y1="64" x2="{W // 2}" y2="{H - 52}" '
    f'stroke="{DIM}" stroke-width="2"/>'
)

# Rose: 16x10 square pixels.
rw = 16 * PX
parts += rects(ROSE, W - 220 - rw // 2, 80, PX, PX)

font = "ui-monospace, SFMono-Regular, Menlo, monospace"
parts += [
    f'<text x="220" y="230" fill="{FG}" font-family="{font}" font-size="21" '
    f'text-anchor="middle">tuxedo</text>',
    f'<text x="220" y="255" fill="{DIM}" font-family="{font}" font-size="15" '
    f'text-anchor="middle">todo.txt</text>',
    f'<text x="{W - 220}" y="230" fill="{FG}" font-family="{font}" font-size="21" '
    f'text-anchor="middle">chiba</text>',
    f'<text x="{W - 220}" y="255" fill="{DIM}" font-family="{font}" font-size="15" '
    f'text-anchor="middle">markdown</text>',
    "</svg>",
]

out = pathlib.Path(__file__).resolve().parent.parent / "docs" / "banner.svg"
out.write_text("\n".join(parts) + "\n")
print("wrote", out)
