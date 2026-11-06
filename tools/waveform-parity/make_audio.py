"""Builds the parity cases' audio from the synthetic demo speakers (deterministic, numpy only).

usage: python make_audio.py <out_dir>
"""

import sys
import wave
from pathlib import Path

import numpy as np

REPO = Path(__file__).resolve().parents[2]
DEMO = REPO / "crates/studio-editor/assets/demo"


def read(path):
    with wave.open(str(path), "rb") as w:
        assert w.getsampwidth() == 2, path
        frames = np.frombuffer(w.readframes(w.getnframes()), dtype="<i2")
        return w.getframerate(), frames.reshape(-1, w.getnchannels()).astype(np.float64)


def write(path, rate, samples):
    samples = np.clip(np.round(samples), -32768, 32767).astype("<i2")
    if samples.ndim == 1:
        samples = samples[:, None]
    with wave.open(str(path), "wb") as w:
        w.setnchannels(samples.shape[1])
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(samples.tobytes())


def resample(samples, rate, target):
    count = int(len(samples) * target / rate)
    t = np.arange(count) * (rate / target)
    return np.stack(
        [np.interp(t, np.arange(len(samples)), samples[:, c]) for c in range(samples.shape[1])],
        axis=1,
    )


def main():
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    rate, s1 = read(DEMO / "speaker_1.wav")
    _, s2 = read(DEMO / "speaker_2.wav")
    # 60 s of speaker 1 (16-76 s) and speaker 2 (1-61 s), each opening on speech.
    a = s1[16 * rate : 76 * rate]
    b = s2[1 * rate : 61 * rate]
    write(out / "s1-full.wav", rate, s1)
    write(out / "s1-60.wav", rate, a)
    write(out / "quiet-60.wav", rate, a * 0.02)
    write(out / "clip-60.wav", rate, a * 8.0)
    write(out / "stereo-60.wav", rate, np.concatenate([a, b], axis=1))
    write(out / "s1-24k.wav", 24_000, resample(a, rate, 24_000))
    write(out / "s1-32k.wav", 32_000, resample(a, rate, 32_000))
    write(out / "s1-48k.wav", 48_000, resample(a, rate, 48_000))
    # Short synthetic fixtures for the Rust behaviour tests (checked in).
    fixtures = REPO / "crates/studio-waveform/tests/fixtures/babel_line"
    fixtures.mkdir(parents=True, exist_ok=True)
    for name, fixture_rate, channels in [
        ("tone-24k-mono", 24_000, 1),
        ("tone-32k-mono", 32_000, 1),
        ("speech-44k-stereo", 44_100, 2),
        ("clip-48k-stereo", 48_000, 2),
        ("tone-8k-mono", 8_000, 1),
    ]:
        length = fixture_rate // 10
        t = np.arange(length) / fixture_rate
        if name.startswith("speech"):
            speech = np.concatenate([a, b], axis=1)[: rate // 10 + rate // 100]
            data = resample(speech, rate, fixture_rate)[:length]
        else:
            # A sweep with a 3.9 kHz partial (above the display low-pass) and a quiet burst.
            tone = 0.6 * np.sin(2 * np.pi * (200 + 2000 * t) * t) + 0.3 * np.sin(
                2 * np.pi * 3900 * t
            )
            tone *= np.where((t > 0.04) & (t < 0.06), 0.05, 1.0)
            data = np.stack([tone * (1 - 0.3 * c) for c in range(channels)], axis=1) * 32767
            if name.startswith("clip"):
                data = data * 3.0
        write(fixtures / f"{name}.wav", fixture_rate, data)


if __name__ == "__main__":
    main()
