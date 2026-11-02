"""Writes src/fonts/*.woff2: Inter and JetBrains Mono cut to what the page sets (spec §1.4).

The weight axis keeps only the range the tokens use, and the glyphs only the characters the
page writes plus printable ASCII, so names and addresses always render in the font.
Characters outside the subset fall back to the next font in the stack; the @font-face
unicode-range in src/lib/tokens.css says the same. Needs fontTools and brotli:

    python3 scripts/fonts.py
"""

from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

WEB = Path(__file__).resolve().parent.parent
SOURCES = WEB / "node_modules" / "@fontsource-variable"

# Printable ASCII and the non-ASCII characters in src (copy, separators, arrows).
UNICODES = [
    *range(0x20, 0x7F),
    0xA0, 0xA7, 0xB7, 0xD7,
    0x2009, 0x2013, 0x2014, 0x2018, 0x2019, 0x201C, 0x201D, 0x2026, 0x203A,
    0x2190, 0x2191, 0x2192, 0x2193, 0x2212, 0x2264, 0x2265,
]
FEATURES = ["kern", "liga", "calt", "ccmp", "locl", "tnum", "zero", "case", "c2sc", "smcp",
            "cv05", "cv08", "ss03"]

FONTS = [
    ("inter/files/inter-latin-wght-normal.woff2", "inter-latin-wght.woff2", (400, 600)),
    ("jetbrains-mono/files/jetbrains-mono-latin-wght-normal.woff2",
     "jetbrains-mono-latin-wght.woff2", (400, 600)),
]


def main() -> None:
    for source, target, (low, high) in FONTS:
        font = TTFont(SOURCES / source, lazy=False)
        options = subset.Options()
        options.flavor = "woff2"
        options.layout_features = FEATURES
        options.name_IDs = [0, 1, 2, 3, 4, 5, 6]
        options.hinting = False
        options.desubroutinize = True
        subsetter = subset.Subsetter(options)
        subsetter.populate(unicodes=UNICODES)
        subsetter.subset(font)
        font = instancer.instantiateVariableFont(font, {"wght": (low, high)})
        out = WEB / "src" / "fonts" / target
        font.flavor = "woff2"
        font.save(out)
        print(f"{target}: {out.stat().st_size} bytes")


if __name__ == "__main__":
    main()
