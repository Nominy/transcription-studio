"""Compares Babel's wavesurfer renders with Studio's (see run.ps1) and writes the report.

usage: python compare.py <outDir>

Levels, so a difference can be attributed:
  signal    Babel's 8 kHz display signal (browser) vs Studio's (Rust), sample by sample
  geometry  canvas outlines vertex by vertex, and per device column the top/bottom extent of
            the filled outline inside the canvas (device px)
  raster    Babel's screenshot vs Studio's outline drawn by the same wavesurfer page: any pixel
            difference here comes from geometry alone
  app       (when <outDir>/app exists) the running Studio's lane crop vs Babel rendering the
            same view: rasterisation (anti-aliasing) and colours
"""

import json
import math
import sys
from pathlib import Path

import numpy as np
from PIL import Image


def load_f32(path):
    return np.fromfile(path, dtype="<f4")


def signal_stats(babel, studio):
    if len(babel) != len(studio):
        return {"length_babel": len(babel), "length_studio": len(studio), "equal": False}
    diff = np.abs(babel.astype(np.float64) - studio.astype(np.float64))
    ulps = np.abs(babel.view("<i4").astype(np.int64) - studio.view("<i4").astype(np.int64))
    return {
        "length": len(babel),
        "differing_samples": int(np.count_nonzero(babel != studio)),
        "max_abs_diff": float(diff.max(initial=0.0)),
        "max_ulps": int(ulps.max(initial=0)),
        "equal": bool(np.array_equal(babel, studio)),
    }


def column_extents(polyline, columns, height, pick):
    """Per device column `c` (covering x in [c, c+1]) the outline's extreme y, clipped."""
    extent = np.full(columns, np.nan)
    for (x0, y0), (x1, y1) in zip(polyline, polyline[1:]):
        if x1 < x0:
            (x0, y0), (x1, y1) = (x1, y1), (x0, y0)
        first = max(int(math.floor(x0)), 0)
        last = min(int(math.ceil(x1)), columns)
        for c in range(first, max(last, first + 1)):
            if c >= columns:
                break
            a = max(x0, c)
            b = min(x1, c + 1)
            if b < a:
                continue
            if x1 == x0:
                ys = [y0, y1]
            else:
                ys = [y0 + (y1 - y0) * (a - x0) / (x1 - x0), y0 + (y1 - y0) * (b - x0) / (x1 - x0)]
            value = pick(ys)
            extent[c] = value if np.isnan(extent[c]) else pick([extent[c], value])
    return np.clip(extent, 0, height)


def viewport_extents(canvases, scroll, dpr, client_width):
    width = int(round(client_width * dpr))
    top = np.full(width, np.nan)
    bottom = np.full(width, np.nan)
    for canvas in canvases:
        offset = int(math.floor((canvas["left"] - scroll) * dpr + 0.5))
        height = canvas["deviceHeight"]
        columns = canvas["deviceWidth"]
        t = column_extents(canvas["top"], columns, height, min)
        b = column_extents(canvas["bottom"], columns, height, max)
        for c in range(columns):
            v = offset + c
            if 0 <= v < width and not np.isnan(t[c]):
                top[v], bottom[v] = t[c], b[c]
    return top, bottom


def outline_stats(babel, studio):
    by_left = {c["left"]: c for c in babel}
    differing = 0
    total = 0
    max_dy = 0.0
    missing = 0
    for canvas in studio:
        other = by_left.get(canvas["left"])
        if other is None or other["top"] is None:
            missing += 1
            continue
        for key in ("top", "bottom"):
            a = np.asarray(other[key], dtype=np.float64)
            b = np.asarray(canvas[key], dtype=np.float64)
            if a.shape != b.shape:
                differing += max(len(a), len(b))
                total += max(len(a), len(b))
                max_dy = math.inf
                continue
            total += len(a)
            rows = np.any(a != b, axis=1)
            differing += int(rows.sum())
            if rows.any():
                max_dy = max(max_dy, float(np.abs(a - b).max()))
    return {"vertices": total, "differing_vertices": differing, "max_vertex_delta": max_dy,
            "canvases_missing_in_babel": missing}


def image_diff(a_path, b_path, heatmap_path, mask=None):
    a = np.asarray(Image.open(a_path).convert("RGB"), dtype=np.int16)
    b = np.asarray(Image.open(b_path).convert("RGB"), dtype=np.int16)
    h = min(a.shape[0], b.shape[0])
    w = min(a.shape[1], b.shape[1])
    a, b = a[:h, :w], b[:h, :w]
    delta = np.abs(a - b).max(axis=2)
    if mask is not None:
        delta = np.where(mask[:h, :w], delta, 0)
    considered = int(mask[:h, :w].sum()) if mask is not None else h * w
    differing = delta > 0
    heat = np.zeros((h, w, 3), dtype=np.uint8)
    gray = (a.mean(axis=2) * 0.35).astype(np.uint8)
    heat[..., 0] = np.where(differing, np.clip(80 + delta * 4, 0, 255), gray).astype(np.uint8)
    heat[..., 1] = np.where(differing, 0, gray)
    heat[..., 2] = np.where(differing, 0, gray)
    Image.fromarray(heat).save(heatmap_path)
    return {
        "size": [w, h],
        "pixels": considered,
        "differing_pixels": int(differing.sum()),
        "differing_percent": 100.0 * float(differing.sum()) / max(considered, 1),
        "differing_over_8_percent": 100.0 * float((delta > 8).sum()) / max(considered, 1),
        "max_channel_delta": int(delta.max(initial=0)),
        "mean_channel_delta_of_differing": float(delta[differing].mean()) if differing.any() else 0.0,
    }


def main():
    out = Path(sys.argv[1])
    cases = json.loads((out / "resolved-cases.json").read_text())
    (out / "heatmaps").mkdir(exist_ok=True)
    report = {"signals": {}, "cases": []}
    for audio in sorted({case["audio"] for case in cases}):
        report["signals"][audio] = signal_stats(
            load_f32(out / "babel" / f"signal-{audio}.f32"),
            load_f32(out / "studio" / f"signal-{audio}.f32"),
        )
    for case in cases:
        babel = json.loads((out / "babel" / f"{case['id']}.json").read_text())
        studio = json.loads((out / "studio" / f"{case['id']}.json").read_text())
        dpr, scroll = case["dpr"], case["scroll"]
        bt, bb = viewport_extents(babel["canvases"], scroll, dpr, case["clientWidth"])
        st, sb = viewport_extents(studio["canvases"], scroll, dpr, case["clientWidth"])
        both = ~np.isnan(bt) & ~np.isnan(st)
        only_one = np.count_nonzero(np.isnan(bt) != np.isnan(st))
        extent_delta = np.concatenate([np.abs(bt - st)[both], np.abs(bb - sb)[both]])
        entry = {
            "id": case["id"],
            "audio": case["audio"],
            "view": {k: case[k] for k in ("clientWidth", "height", "dpr", "pxPerSec", "scroll", "barHeight")},
            "outline": outline_stats(babel["canvases"], studio["canvases"]),
            "columns": {
                "compared": int(both.sum()),
                "only_one_side": int(only_one),
                "max_extent_delta_px": float(extent_delta.max(initial=0.0)),
                "mean_extent_delta_px": float(extent_delta.mean()) if extent_delta.size else 0.0,
                "differing_columns": int(np.count_nonzero((np.abs(bt - st) > 0)[both] | (np.abs(bb - sb) > 0)[both])),
            },
            "raster_geometry": image_diff(
                out / "babel" / f"{case['id']}.png",
                out / "studio-geometry" / f"{case['id']}.png",
                out / "heatmaps" / f"{case['id']}-geometry.png",
            ),
        }
        report["cases"].append(entry)
    app_report = out / "app" / "app-report.json"
    if app_report.exists():
        report["app"] = json.loads(app_report.read_text())
    (out / "report.json").write_text(json.dumps(report, indent=2))
    write_markdown(out, report)


def write_markdown(out, report):
    lines = ["# Waveform parity report", "", "## Display signal (8 kHz)", "",
             "| audio | samples | differing | max abs diff | max ulps |", "|---|---|---|---|---|"]
    for audio, s in report["signals"].items():
        lines.append(f"| {audio} | {s.get('length', '?')} | {s.get('differing_samples', '?')} | "
                     f"{s.get('max_abs_diff', '?')} | {s.get('max_ulps', '?')} |")
    lines += ["", "## Geometry and geometry-only raster", "",
              "| case | vertices differing | columns compared | max extent Δ px | mean extent Δ px | raster differing % | max channel Δ | heatmap |",
              "|---|---|---|---|---|---|---|---|"]
    for c in report["cases"]:
        o, col, r = c["outline"], c["columns"], c["raster_geometry"]
        lines.append(
            f"| {c['id']} | {o['differing_vertices']}/{o['vertices']} | {col['compared']} | "
            f"{col['max_extent_delta_px']:.3f} | {col['mean_extent_delta_px']:.4f} | "
            f"{r['differing_percent']:.4f} | {r['max_channel_delta']} | heatmaps/{c['id']}-geometry.png |")
    if "app" in report:
        lines += ["", "## Running app vs Babel (same view, region edges/labels and playhead masked)", "",
                  "| case | registration dx,dy | differing % | >8 % | >32 % | max Δ | columns | max extent Δ px | mean extent Δ px | interior colour Babel / Studio | heatmap |",
                  "|---|---|---|---|---|---|---|---|---|---|---|"]
        for r in report["app"]:
            lines.append(
                f"| {r['id']} | {r['registration_px']} | {r['differing_percent']:.3f} | "
                f"{r['differing_over_8_percent']:.3f} | {r['differing_over_32_percent']:.3f} | "
                f"{r['max_channel_delta']} | {r['columns_compared']} | {r['column_extent_max_px']:.0f} | "
                f"{r['column_extent_mean_px']:.4f} | {r['interior_colour_babel']} / {r['interior_colour_studio']} | "
                f"heatmaps/app-{r['id']}.png |")
    (out / "report.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))


if __name__ == "__main__":
    main()
