"""Running-app side of the parity benchmark (see app.ps1, run.ps1 -App).

usage: python app_compare.py prepare <outDir>   lane geometry -> app/babel-cases.json
       python app_compare.py compare <outDir>   app vs Babel -> app/app-report.json, heatmaps

`prepare` measures the timeline lanes in the opening-view capture: the first lane (Speaker 1)
is silent there, so its 2 px centre line gives the lane's middle; the lane body is the run of
lane-background pixels left of the timeline's right border. Every captured state has the
playhead at 0 s and the viewport starting at 0 s; its duration is the lane width (CSS px)
over zoom x 200 px/s. Babel then renders each lane at that view on Studio's lane background
with Studio's region fills underneath (wavesurfer has no other decoration in the comparison).

`compare` registers each Babel render on the app capture (+-2 device px, the measurement's
tolerance) and reports, outside region edges/labels and the playhead: per device column the
top/bottom extent of the waveform (pixels at least half waveform colour over the known
background) and pixel differences.
"""

import json
import math
import sys
from pathlib import Path

import numpy as np
from PIL import Image

REPO = Path(__file__).resolve().parents[2]
DEMO = REPO / "crates/studio-editor/assets/demo"
SAMPLE = REPO / "crates/studio-editor/assets/sample-task.json"
WAVE = np.array([0x70, 0x70, 0x70], dtype=np.float64)
LANES = [  # Studio lane order and colours (LaneColor::for_lane_index), Babel's speaker colours
    {"speaker": "speaker-1", "audio": "speaker_1.wav", "color": [0x64, 0xB5, 0xF6]},
    {"speaker": "speaker-2", "audio": "speaker_2.wav", "color": [0xB0, 0x83, 0xFF]},
]
LANE_CSS = 150.0
CHIP_CSS = (260.0, 24.0)  # region label / timestamp chip area masked at a region's start
EDGE_PX = 4  # region edge lines and the playhead, device px each side
OUTLINE_CSS = 3.0  # Studio's region top/bottom outline (selection border), masked


def wav_duration(path):
    import wave

    with wave.open(str(path), "rb") as w:
        return w.getnframes() / w.getframerate()


def segments():
    task = json.loads(SAMPLE.read_text(encoding="utf-8"))
    rows = sorted(task["annotations"], key=lambda a: a["start_seconds"])
    return [
        {"speaker": a["speaker_key"], "start": a["start_seconds"], "end": a["end_seconds"],
         "selected": i == 0}
        for i, a in enumerate(rows)
    ]


def measure(image, dpr):
    pixels = np.asarray(image.convert("RGB"), dtype=np.int16)
    h, w, _ = pixels.shape
    x = int(w * 0.6)
    column = pixels[:, x]
    hits = np.nonzero(np.all(column == 112, axis=1))[0]
    centre = int(hits[0])  # first row of the silent lane's 2 px centre line
    device_h = int(math.floor(LANE_CSS * dpr + 0.5))
    top = centre + 1 - device_h // 2
    bg = pixels[top + 3, x]
    row = pixels[top + 3]
    is_bg = np.all(np.abs(row - bg) <= 2, axis=1)
    right = x
    while right < w and is_bg[right]:
        right += 1
    left = x
    while left > 0 and is_bg[left - 1]:
        left -= 1
    # The playhead line (at 0 s) covers the lane's first device column(s); the lane starts
    # right before the background run, within registration tolerance.
    left -= 1
    return {"left": left, "right": right, "top": top, "device_height": device_h,
            "background": bg.tolist(), "lane_step": device_h + 1}


def prepare(out):
    app = out / "app"
    states = json.loads((app / "states.json").read_text(encoding="utf-8-sig"))
    dpr = float(states["dpr"])
    geometry = measure(Image.open(app / "zoom1.png"), dpr)
    width_css = (geometry["right"] - geometry["left"]) / dpr
    bg = geometry["background"]
    cases = []
    for state in states["states"]:
        zoom = float(state["zoom"])
        bounds = max(wav_duration(DEMO / lane["audio"]) for lane in LANES)
        duration = min(width_css / (zoom * 200.0), bounds)
        for index, lane in enumerate(LANES):
            underlay = []
            for seg in segments():
                if seg["speaker"] != lane["speaker"] or seg["end"] < 0 or seg["start"] > duration:
                    continue
                alpha = 0.26 if seg["selected"] else 0.10
                x0 = seg["start"] / duration * width_css
                x1 = seg["end"] / duration * width_css
                underlay.append({"left": x0, "width": x1 - x0,
                                 "color": f"rgba({lane['color'][0]},{lane['color'][1]},{lane['color'][2]},{alpha})",
                                 "start": seg["start"], "end": seg["end"]})
            cases.append({
                "id": f"{state['id']}-lane{index + 1}",
                "state": state["id"],
                "lane": index,
                "audio": lane["audio"],
                "clientWidth": width_css,
                "height": LANE_CSS,
                "dpr": dpr,
                "pxPerSec": width_css / duration,
                "scroll": 0,
                "barHeight": 20,
                "progress": 0,
                "waveColor": "#707070",
                "progressColor": "#{:02X}{:02X}{:02X}".format(*lane["color"]),
                "background": "rgb({},{},{})".format(*bg),
                "underlay": underlay,
                "viewSeconds": duration,
            })
    (app / "geometry.json").write_text(json.dumps(geometry, indent=2))
    (app / "babel-cases.json").write_text(json.dumps(cases, indent=2))
    print(f"lane {geometry} width {width_css:.3f} css px at dpr {dpr}")


def coverage(pixels, background):
    """Fraction of waveform colour over the per-column background (0..1)."""
    span = WAVE[None, None, :] - background[None, :, :]
    norm = np.maximum((span * span).sum(axis=2), 1.0)
    return ((pixels - background[None, :, :]) * span).sum(axis=2) / norm


def extents(cov, mask_columns):
    wave = cov >= 0.5
    any_wave = wave.any(axis=0)
    top = np.where(any_wave, wave.argmax(axis=0), -1)
    bottom = np.where(any_wave, wave.shape[0] - 1 - wave[::-1].argmax(axis=0), -1)
    return top, bottom, any_wave & ~mask_columns


def compare(out):
    app = out / "app"
    geometry = json.loads((app / "geometry.json").read_text())
    cases = json.loads((app / "babel-cases.json").read_text())
    (out / "heatmaps").mkdir(exist_ok=True)
    results = []
    for case in cases:
        dpr = case["dpr"]
        babel = np.asarray(Image.open(app / f"{case['id']}-babel.png").convert("RGB"), dtype=np.float64)
        h, w, _ = babel.shape
        shot = np.asarray(Image.open(app / f"{case['state']}.png").convert("RGB"), dtype=np.float64)
        top0 = geometry["top"] + case["lane"] * geometry["lane_step"]
        # Background per column: Studio's lane background with the region fills composited.
        bg = np.tile(np.array(geometry["background"], dtype=np.float64), (w, 1))
        mask = np.zeros((h, w), dtype=bool)
        mask[:, : EDGE_PX + 1] = True  # playhead at 0 s
        for region in case["underlay"]:
            colour = np.array([int(v) for v in region["color"][5:-1].split(",")[:3]], dtype=np.float64)
            alpha = float(region["color"][5:-1].split(",")[3])
            x0 = int(round(region["left"] * dpr))
            x1 = int(round((region["left"] + region["width"]) * dpr))
            a, b = max(x0, 0), min(x1, w)
            if a < b:
                bg[a:b] = np.round(bg[a:b] * (1 - alpha) + colour * alpha)
            # Studio outlines regions at their (view-clamped) edges.
            for edge in (max(x0, 0), min(x1, w - 1)):
                mask[:, max(edge - EDGE_PX, 0): max(min(edge + EDGE_PX + 1, w), 0)] = True
            if 0 <= x0 < w:
                cw, ch = int(CHIP_CSS[0] * dpr), int(CHIP_CSS[1] * dpr)
                mask[:ch, x0: x0 + cw] = True
                mask[h - ch:, x0: x0 + cw] = True
            if 0 < x1 <= w:  # the selected region's end timestamp chip
                cw, ch = int(CHIP_CSS[0] * dpr), int(CHIP_CSS[1] * dpr)
                mask[h - ch:, max(x1 - cw, 0): x1] = True
            if a < b:
                rows = int(math.ceil(OUTLINE_CSS * dpr))
                mask[:rows, a:b] = True
                mask[h - rows:, a:b] = True
        best = None
        for dy in range(-2, 3):
            for dx in range(-2, 3):
                y, x = top0 + dy, geometry["left"] + dx
                crop = shot[y: y + h, x: x + w]
                if crop.shape != babel.shape:
                    continue
                score = np.abs(crop - babel)[~mask].mean()
                if best is None or score < best[0]:
                    best = (score, dx, dy, crop)
        _, dx, dy, crop = best
        delta = np.abs(crop - babel).max(axis=2)
        delta[mask] = 0
        considered = int((~mask).sum())
        mask_columns = mask.all(axis=0) | mask[h // 2]
        # Masked rows (region outlines, label chips) count as background on both sides.
        cov_babel = np.where(mask, 0.0, coverage(babel, bg))
        cov_studio = np.where(mask, 0.0, coverage(crop, bg))
        bt, bb, bvalid = extents(cov_babel, mask_columns)
        st, sb, svalid = extents(cov_studio, mask_columns)
        both = bvalid & svalid
        ext = np.concatenate([np.abs(bt - st)[both], np.abs(bb - sb)[both]]).astype(np.float64)
        interior = (cov_babel > 0.99) & (cov_studio > 0.99)
        heat = np.zeros((h, w, 3), dtype=np.uint8)
        gray = (babel.mean(axis=2) * 0.35).astype(np.uint8)
        diff = delta > 0
        heat[..., 0] = np.where(diff, np.clip(80 + delta * 4, 0, 255), gray)
        heat[..., 1] = np.where(diff, 0, gray)
        heat[..., 2] = np.where(mask, 90, np.where(diff, 0, gray))
        Image.fromarray(heat).save(out / "heatmaps" / f"app-{case['id']}.png")
        Image.fromarray(crop.astype(np.uint8)).save(app / f"{case['id']}-studio.png")
        results.append({
            "id": case["id"],
            "registration_px": [dx, dy],
            "pixels": considered,
            "differing_percent": 100.0 * float((delta > 0).sum()) / considered,
            "differing_over_8_percent": 100.0 * float((delta > 8).sum()) / considered,
            "differing_over_32_percent": 100.0 * float((delta > 32).sum()) / considered,
            "max_channel_delta": int(delta.max()),
            "columns_compared": int(both.sum()),
            "column_extent_max_px": float(ext.max(initial=0.0)),
            "column_extent_mean_px": float(ext.mean()) if ext.size else 0.0,
            "columns_differing": int(((bt != st) | (bb != sb))[both].sum()),
            # Edge pixels only: mean signed waveform coverage Studio - Babel. ~0 means the
            # two rasterisers spread the same edges differently (anti-aliasing); a bias would
            # mean an offset or a fill/outline difference.
            "edge_coverage_bias": float((cov_studio - cov_babel)[(delta > 0) & ~mask].mean())
            if ((delta > 0) & ~mask).any() else 0.0,
            "edge_coverage_mean_abs": float(np.abs(cov_studio - cov_babel)[(delta > 0) & ~mask].mean())
            if ((delta > 0) & ~mask).any() else 0.0,
            "interior_colour_babel": babel[interior].mean(axis=0).round(2).tolist() if interior.any() else None,
            "interior_colour_studio": crop[interior].mean(axis=0).round(2).tolist() if interior.any() else None,
        })
        print(json.dumps(results[-1]))
    (app / "app-report.json").write_text(json.dumps(results, indent=2))


if __name__ == "__main__":
    {"prepare": prepare, "compare": compare}[sys.argv[1]](Path(sys.argv[2]))
