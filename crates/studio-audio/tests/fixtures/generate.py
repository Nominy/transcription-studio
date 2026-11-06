"""Regenerate tiny, original synthetic audio fixtures (no third-party recordings).

Requires FFmpeg with libmp3lame, libvorbis, libopus and libvpx encoders.
Every fixture is 0.25 seconds of stereo 48 kHz audio: 440 Hz left, 880 Hz
right, amplitude 0.25. Video fixtures add a 16x16 black video track.
The encoded bytes are committed; tests never require FFmpeg. FFmpeg version
and codec version changes may change encoded bytes, not the signal contract.
The generated signals and fixture files are dedicated to CC0-1.0.
Run: python crates/studio-audio/tests/fixtures/generate.py
"""
from pathlib import Path
import subprocess

ROOT = Path(__file__).parent
command = [
    "ffmpeg", "-hide_banner", "-loglevel", "error", "-nostdin", "-y",
    "-f", "lavfi", "-i",
    "aevalsrc=0.25*sin(2*PI*440*t)|0.25*sin(2*PI*880*t):s=48000:d=0.25",
    "-f", "lavfi", "-i", "color=c=black:s=16x16:r=8:d=0.25",
]
for name, codec, container in [
    ("tone.flac", "flac", "flac"),
    ("tone.mp3", "libmp3lame", "mp3"),
    ("tone-vorbis.ogg", "libvorbis", "ogg"),
    ("tone-opus.ogg", "libopus", "ogg"),
    ("tone.m4a", "aac", "ipod"),
]:
    command += ["-map", "0:a", "-c:a", codec, "-map_metadata", "-1",
                "-fflags", "+bitexact", "-flags:a", "+bitexact",
                "-f", container, str(ROOT / name)]
for name, audio, video, container in [
    ("tone-video.mp4", "aac", "mpeg4", "mp4"),
    ("tone-video.webm", "libvorbis", "libvpx", "webm"),
    ("tone-video.mkv", "flac", "mpeg4", "matroska"),
]:
    command += ["-map", "1:v", "-map", "0:a", "-c:a", audio,
                "-c:v", video, "-threads", "1", "-map_metadata", "-1",
                "-fflags", "+bitexact", "-flags:a", "+bitexact",
                "-flags:v", "+bitexact", "-f", container, str(ROOT / name)]
command += ["-map", "1:v", "-c:v", "mpeg4", "-threads", "1",
            "-fflags", "+bitexact", "-flags:v", "+bitexact",
            str(ROOT / "video-only.mp4")]
command += ["-map", "0:a", "-c:a", "aac", "-fflags", "+bitexact",
            "-flags:a", "+bitexact", "-encryption_scheme", "cenc-aes-ctr",
            "-encryption_key", "00000000000000000000000000000000",
            "-encryption_kid", "00000000000000000000000000000000",
            str(ROOT / "encrypted.m4a")]
# This is a bounded fixture encoder, not a build or test run.
subprocess.run(command, check=True, timeout=30)
