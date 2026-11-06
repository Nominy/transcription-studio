#!/usr/bin/env python
"""Renders every derived Exaudio brand asset from the three hand-drawn sources.

Sources (hand-edited, committed):
  assets/brand/exaudio-mark.svg       ring mark, ink on transparent
  assets/brand/exaudio-mark-dark.svg  ring mark, paper on transparent
  assets/brand/exaudio-icon.svg       mark on an ink tile (app icon)

Outputs (generated, committed; never hand-edit):
  assets/brand/exaudio-wordmark.svg       "exaudio" outlined from Outfit Regular, ink
  assets/brand/exaudio-wordmark-dark.svg  same, paper (for dark surfaces)
  assets/brand/header-light.svg           icon tile + ink wordmark, 580x184 (README, site)
  assets/brand/header-dark.svg            icon tile + paper wordmark, 580x184
  assets/brand/social-preview.svg/.png    1280x640 ink card: icon, wordmark, tagline
  assets/app/exaudio.ico                  16/24/32/48/256 frames of the icon tile
  assets/app/exaudio-256.png, exaudio-512.png, exaudio-mark.png (224 px)
  site/assets/header-*.svg, exaudio-icon.svg  copies the site serves locally

Requirements: Python 3 with fontTools (varLib instancer), ImageMagick `magick` on PATH
(rsvg delegate), and the Outfit variable font (SIL OFL 1.1), located through
EXAUDIO_OUTFIT_TTF or `--font`. Type is outlined so nothing downstream needs the font.

    python tools/brand/render.py --font "path/to/Outfit[wght].ttf"
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

REPO = Path(__file__).resolve().parents[2]
BRAND = REPO / "assets" / "brand"
APP = REPO / "assets" / "app"
SITE_ASSETS = REPO / "site" / "assets"

INK = "#14161C"
PAPER = "#F4F1EA"
AMBER = "#D99A2B"

WORDMARK = "exaudio"
WORDMARK_TRACKING = -0.01  # em; "about -1 %"
TAGLINE = "From audio, every word."
ICO_SIZES = (16, 24, 32, 48, 256)


class Typesetter:
    """Outlines strings from one static instance of the variable font."""

    def __init__(self, font_path: Path, weight: float = 400):
        variable = TTFont(font_path)
        self.font = instancer.instantiateVariableFont(variable, {"wght": weight}, inplace=False)
        self.upem = self.font["head"].unitsPerEm
        self.cmap = self.font.getBestCmap()
        self.glyphs = self.font.getGlyphSet()
        self.hmtx = self.font["hmtx"]
        self._kern = self._pair_lookups()

    def _pair_lookups(self):
        gpos = self.font["GPOS"].table
        indices = []
        for record in gpos.FeatureList.FeatureRecord:
            if record.FeatureTag == "kern":
                indices.extend(record.Feature.LookupListIndex)
        lookups = [gpos.LookupList.Lookup[i] for i in sorted(set(indices))]
        subtables = []
        for lookup in lookups:
            for subtable in lookup.SubTable:
                if lookup.LookupType == 9:
                    subtable = subtable.ExtSubTable
                if getattr(subtable, "LookupType", 2) == 2:
                    subtables.append(subtable)
        return subtables

    def kern(self, left: str, right: str) -> float:
        for subtable in self._kern:
            coverage = subtable.Coverage.glyphs
            if left not in coverage:
                continue
            if subtable.Format == 1:
                for record in subtable.PairSet[coverage.index(left)].PairValueRecord:
                    if record.SecondGlyph == right:
                        return getattr(record.Value1, "XAdvance", 0) or 0
            elif subtable.Format == 2:
                class1 = subtable.ClassDef1.classDefs.get(left, 0)
                class2 = subtable.ClassDef2.classDefs.get(right, 0)
                record = subtable.Class1Record[class1].Class2Record[class2]
                value = getattr(record.Value1, "XAdvance", 0) if record.Value1 else 0
                if value:
                    return value
        return 0

    def outline(self, text: str, tracking_em: float = 0.0):
        """One SVG path in font units, y up, pen at x=0; returns (d, bounds, advance)."""
        pen = SVGPathPen(self.glyphs)
        bounds = BoundsPen(self.glyphs)
        x = 0.0
        tracking = tracking_em * self.upem
        names = [self.cmap[ord(ch)] for ch in text]
        for index, name in enumerate(names):
            glyph = self.glyphs[name]
            shifted = TransformPen(pen, (1, 0, 0, 1, x, 0))
            glyph.draw(shifted)
            glyph.draw(TransformPen(bounds, (1, 0, 0, 1, x, 0)))
            x += self.hmtx[name][0] + tracking
            if index + 1 < len(names):
                x += self.kern(name, names[index + 1])
        return pen.getCommands(), bounds.bounds, x


def fmt(value: float) -> str:
    return f"{value:.5f}".rstrip("0").rstrip(".")


def path_element(d: str, fill: str, scale: float, tx: float, ty: float) -> str:
    """Font-unit path placed with its origin at (tx, ty) in SVG space, y flipped."""
    return (
        f'<path fill="{fill}" transform="translate({fmt(tx)} {fmt(ty)}) '
        f'scale({fmt(scale)} {fmt(-scale)})" d="{d}"/>'
    )


def icon_group(scale: float, tx: float, ty: float, on_dark: bool = False) -> str:
    """The icon tile's drawing, inlined so headers render anywhere without a file reference.

    On dark surfaces the ink tile gets a faint paper hairline so its edge still reads.
    """
    source = (BRAND / "exaudio-icon.svg").read_text(encoding="utf-8")
    inner = re.search(r"<svg[^>]*>(.*)</svg>", source, re.S).group(1)
    inner = re.sub(r"<title>.*?</title>|<!--.*?-->", "", inner, flags=re.S)
    body = "\n".join(line for line in inner.splitlines() if line.strip())
    if on_dark:
        tile = re.search(r'<rect [^>]*rx="[^"]*"', body).group(0)
        body += "\n" + re.sub(r' fill="[^"]*"', "", tile) + (
            f' fill="none" stroke="{PAPER}" stroke-opacity="0.22" stroke-width="1.5"/>'
        )
    return f'<g transform="translate({fmt(tx)} {fmt(ty)}) scale({fmt(scale)})">\n{body}\n</g>'


def svg(width: float, height: float, title: str, body: str) -> str:
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {fmt(width)} {fmt(height)}" '
        f'width="{fmt(width)}" height="{fmt(height)}">\n'
        f"  <title>{title}</title>\n"
        f"  <!-- Generated by tools/brand/render.py; edit the sources, not this file. -->\n"
        f"{body}\n</svg>\n"
    )


def write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8", newline="\n")
    print(f"wrote {path.relative_to(REPO)}")


def magick(*args: str) -> None:
    subprocess.run(["magick", *args], check=True)


def render_svg_png(source: Path, target: Path, size: int | None = None, width: int | None = None) -> None:
    """rsvg rasterises at 96 dpi over the viewBox; scale density for exact pixel sizes."""
    view = re.search(r'viewBox="0 0 ([\d.]+) ([\d.]+)"', source.read_text(encoding="utf-8"))
    box_w = float(view.group(1))
    target_w = size if size is not None else width if width is not None else box_w
    density = 96.0 * target_w / box_w
    magick("-background", "none", "-density", fmt(density), str(source), "-depth", "8", "-strip", str(target))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--font", default=os.environ.get("EXAUDIO_OUTFIT_TTF"), help="Outfit[wght].ttf")
    args = parser.parse_args()
    if not args.font:
        parser.error("--font or EXAUDIO_OUTFIT_TTF is required")
    if shutil.which("magick") is None:
        parser.error("ImageMagick `magick` is not on PATH")

    type_ = Typesetter(Path(args.font))
    upem = type_.upem

    # Wordmark: tight to its ink, 64 units tall so layouts size it by height.
    word_d, (wx0, wy0, wx1, wy1) = type_.outline(WORDMARK, WORDMARK_TRACKING)[:2]
    word_scale = 64.0 / (wy1 - wy0)
    word_w = (wx1 - wx0) * word_scale
    word_box = (word_w, 64.0)
    for name, fill in (("exaudio-wordmark.svg", INK), ("exaudio-wordmark-dark.svg", PAPER)):
        body = "  " + path_element(word_d, fill, word_scale, -wx0 * word_scale, wy1 * word_scale)
        write(BRAND / name, svg(word_box[0], word_box[1], "exaudio", body))
    print(f"wordmark aspect {word_w / 64.0:.4f} (width/height)")

    # Headers: 580x184 box, tile 136 px at the left, wordmark as large as the remaining
    # width allows (ascender capped at 96 px) with its x-height centred on the tile.
    box_w, box_h, margin, tile_px, gap = 580.0, 184.0, 8.0, 136.0, 32.0
    x_height = type_.font["OS/2"].sxHeight
    text_left = margin + tile_px + gap
    header_scale = min(96.0 / wy1, (box_w - margin - text_left) / (wx1 - wx0))
    baseline = box_h / 2 + x_height / 2 * header_scale
    text_x = text_left - wx0 * header_scale
    tile_y = (box_h - tile_px) / 2
    for name, fill, on_dark in (("header-light.svg", INK, False), ("header-dark.svg", PAPER, True)):
        body = (
            "  " + icon_group(tile_px / 64.0, margin, tile_y, on_dark)
            + "\n  " + path_element(word_d, fill, header_scale, text_x, baseline)
        )
        write(BRAND / name, svg(box_w, box_h, "Exaudio", body))

    # Social preview: ink card, icon + wordmark centred, tagline beneath.
    tag_d, (tx0, ty0, tx1, ty1), _ = type_.outline(TAGLINE)
    card_w, card_h = 1280, 640
    tile = 176.0
    gap = 44.0
    word_h = 118.0  # ascender height
    social_scale = word_h / wy1
    word_ink_w = (wx1 - wx0) * social_scale
    group_w = tile + gap + word_ink_w
    left = (card_w - group_w) / 2
    tile_y = 182.0
    word_baseline = tile_y + tile / 2 + x_height / 2 * social_scale
    tag_scale = 40.0 / upem
    tag_w = (tx1 - tx0) * tag_scale
    tag_baseline = tile_y + tile + 96
    body = "\n".join(
        [
            f'  <rect width="{card_w}" height="{card_h}" fill="{INK}"/>',
            "  " + icon_group(tile / 64.0, left, tile_y, on_dark=True),
            "  " + path_element(word_d, PAPER, social_scale, left + tile + gap - wx0 * social_scale, word_baseline),
            "  " + path_element(tag_d, AMBER, tag_scale, (card_w - tag_w) / 2 - tx0 * tag_scale, tag_baseline),
        ]
    )
    social_svg = BRAND / "social-preview.svg"
    write(social_svg, svg(card_w, card_h, f"Exaudio — {TAGLINE}", body))
    render_svg_png(social_svg, BRAND / "social-preview.png", width=card_w)
    print("wrote assets/brand/social-preview.png")

    # App icon rasters from the tile.
    icon = BRAND / "exaudio-icon.svg"
    with tempfile.TemporaryDirectory() as tmp:
        frames = []
        for size in ICO_SIZES:
            frame = Path(tmp) / f"{size}.png"
            render_svg_png(icon, frame, size=size)
            frames.append(str(frame))
        magick(*frames, str(APP / "exaudio.ico"))
    print("wrote assets/app/exaudio.ico")
    for size, name in ((256, "exaudio-256.png"), (512, "exaudio-512.png"), (224, "exaudio-mark.png")):
        render_svg_png(icon, APP / name, size=size)
        print(f"wrote assets/app/{name}")

    # Site copies (pages.yml copies the same files into _site/assets on deploy).
    for name in ("header-light.svg", "header-dark.svg", "exaudio-icon.svg"):
        shutil.copyfile(BRAND / name, SITE_ASSETS / name)
        print(f"wrote site/assets/{name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
