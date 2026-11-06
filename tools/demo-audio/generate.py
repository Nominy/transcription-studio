"""Synthesises Exaudio's two-speaker demo task: an original Russian dialogue spoken by two
Piper voices, one speaker per channel file, with annotations timed to the synthesised speech.

usage: python tools/demo-audio/generate.py <voices_dir>

<voices_dir> holds the Piper voices ru_RU-denis-medium and ru_RU-dmitri-medium (.onnx +
.onnx.json); fetch them with `python -m piper.download_voices ru_RU-denis-medium
ru_RU-dmitri-medium` (pip install piper-tts). Both voices are trained on CC0 datasets
(https://github.com/OHF-Voice/voice-datasets), so the generated audio is freely redistributable.

Writes crates/studio-editor/assets/demo/speaker_{1,2}.wav (44.1 kHz mono PCM16, each speaker's
voice alone on its own track), sample-task.json for Babel codec regressions, and native
sample-project.json for the extension-free Local demo. Piper samples noise, so each run
produces slightly different audio and timings; transcript-derived tests do not depend on audio.
"""

import json
import re
import sys
import uuid
import wave
from pathlib import Path

import numpy as np
from piper import PiperVoice, SynthesisConfig
from scipy.signal import resample_poly

REPO = Path(__file__).resolve().parents[2]
DEMO_DIR = REPO / "crates/studio-editor/assets/demo"
SAMPLE_TASK = REPO / "crates/studio-editor/assets/sample-task.json"
SAMPLE_PROJECT = REPO / "crates/studio-editor/assets/sample-project.json"
RATE = 44_100
NAMESPACE = uuid.uuid5(uuid.NAMESPACE_URL, "https://github.com/Nominy/exaudio/demo")

# Speaker 1 answers in Piper's "denis" voice, speaker 2 asks in "dmitri".
VOICES = {"speaker-1": ("ru_RU-denis-medium", 1.0), "speaker-2": ("ru_RU-dmitri-medium", 0.95)}

# Turns of the main speaker; each sentence may be followed by the other speaker's backchannel,
# which overlaps the start of the next sentence. Transcript text follows Babel's Russian
# conventions: fillers, false starts and numbers as `digits {СКАЗ: spoken form}`.
DIALOGUE = [
    ("speaker-2", [
        ("Слушай, э, я тут давно хотел тебя спросить, а, по поводу похода.", None),
        ("Мы с друзьями решили летом наконец-то сплавиться на байдарках, но никто из нас, ну, толком этого никогда не делал.", "Да."),
        ("И вот я сижу, смотрю маршруты, и как-то голова кругом идёт, честно говоря.", None),
    ]),
    ("speaker-1", [
        ("Да, конечно, давай разберёмся. Это вообще отличная идея, особенно для первого раза.", "Давай."),
        ("Смотри, я бы начал с чего-нибудь спокойного, а, без порогов, чтобы вы просто привыкли к лодке.", "Логично."),
        ("Э, и сразу вопрос: вас сколько человек вообще собирается?", None),
    ]),
    ("speaker-2", [
        ("А, ну, нас пока 4 {СКАЗ: четыре} человека, но, может, ещё кто-то подтянется.", "Ага."),
        ("По времени у нас есть 3 {СКАЗ: три} дня в конце июля, это прямо железно.", "Да, хорошо."),
        ("И бюджет, а, где-то 15 {СКАЗ: пятнадцать} тысяч на человека, вместе с дорогой.", "Да, понял."),
        ("Палатки у нас свои есть, а вот байдарок, конечно, нет.", "Угу."),
        ("Ну и, э, хотелось бы, чтобы было красиво, а не просто грести целый день.", "Понял, да, хорошо."),
    ]),
    ("speaker-1", [
        ("Слушай, ну, под ваши условия отлично подходит какая-нибудь спокойная река недалеко от города.", "Так."),
        ("Байдарки можно  взять в прокате, обычно это выходит 2 {СКАЗ: две} тысячи в сутки за лодку.", None),
        ("За 3 {СКАЗ: три} дня спокойно пройдёте километров 40 {СКАЗ: сорок}, и ещё останется время на костёр.", None),
    ]),
    ("speaker-2", [
        ("О, это звучит здорово. А, ты знаешь, я вот чего боюсь: мы же ни разу не собирали байдарку.", "Да, понял."),
        ("Это, э, сложно вообще, или там за- за полчаса разберёмся?", "Да нет."),
        ("И ещё вопрос, а, что делать, если пойдёт дождь все 3 {СКАЗ: три} дня?", "Ага."),
        ("Не хочется, чтобы первый поход закончился тем, что все промокли и поругались.", "Понимаю, да."),
    ]),
    ("speaker-1", [
        ("Да, конечно. Ну, во-первых, собирать лодку несложно, в прокате обычно всё показывают.", "Ясно."),
        ("Во-вторых, гермомешки берите обязательно, а, это прямо главное правило.", "Точно."),
        ("Ну и дождь, э, это не страшно, если тент натянуть заранее, ещё до того, как начнёт капать.", "Ну, вот как."),
        ("А вообще, я бы на вашем месте выбрал реку, где по берегам есть стоянки, чтобы не искать место в темноте.", "Понятно."),
        ("Хочешь, я тебе вечером скину пару маршрутов, а вы уже сами выберете?", "Давай, спасибо."),
    ]),
]

SENTENCE_PAUSE = 0.42  # main speaker: speech end -> next sentence
BACKCHANNEL_DELAY = 0.12  # main speech end -> backchannel speech start
BACKCHANNEL_PAUSE = 0.55  # main speech end -> next sentence when a backchannel sits between
TURN_GAP = 0.38  # last speech of a turn -> first speech of the next
ROW_PAD = 0.12  # row bounds around a turn's speech
BACKCHANNEL_PAD = 0.08


def spoken(text):
    """What the voice says for a transcript line: numbers read out, whitespace collapsed."""
    text = re.sub(r"\d+ \{СКАЗ: ([^}]+)\}", r"\1", text)
    return re.sub(r"\s+", " ", text).strip()


def synthesize(voice, length_scale, text):
    chunks = list(voice.synthesize(spoken(text), syn_config=SynthesisConfig(length_scale=length_scale)))
    audio = np.concatenate([chunk.audio_float_array for chunk in chunks]).astype(np.float64)
    audio = resample_poly(audio, RATE, chunks[0].sample_rate)
    # Speech bounds: first/last 10 ms frame within 40 dB of the clip's loudest frame.
    frame = RATE // 100
    rms = np.sqrt(np.mean(audio[: len(audio) // frame * frame].reshape(-1, frame) ** 2, axis=1))
    loud = np.nonzero(rms > rms.max() * 0.01)[0]
    speech = audio[loud[0] * frame : (loud[-1] + 1) * frame]
    return speech / np.sqrt(np.mean(speech**2)) * 0.07  # equal loudness, peaks below full scale


def place(tracks, speaker, start, clip):
    offset = round(start * RATE)
    track = tracks[speaker]
    if len(track) < offset + len(clip):
        track.resize(offset + len(clip) + RATE, refcheck=False)
    track[offset : offset + len(clip)] += clip
    return start + len(clip) / RATE


def row(speaker, start, end, text):
    return {"speaker": speaker, "start": round(start, 3), "end": round(end, 3), "text": text}


def build(voices_dir):
    voices = {
        speaker: (PiperVoice.load(str(Path(voices_dir) / f"{name}.onnx")), scale)
        for speaker, (name, scale) in VOICES.items()
    }
    tracks = {speaker: np.zeros(RATE * 10) for speaker in VOICES}
    rows = []
    cursor = 0.9
    for speaker, sentences in DIALOGUE:
        other = "speaker-1" if speaker == "speaker-2" else "speaker-2"
        turn_start = cursor
        turn_text = []
        for index, (sentence, backchannel) in enumerate(sentences):
            end = place(tracks, speaker, cursor, synthesize(*voices[speaker], sentence))
            turn_text.append(sentence)
            last = index == len(sentences) - 1
            if backchannel:
                bc_start = end + BACKCHANNEL_DELAY
                bc_end = place(tracks, other, bc_start, synthesize(*voices[other], backchannel))
                rows.append(row(other, bc_start - BACKCHANNEL_PAD, bc_end + BACKCHANNEL_PAD, backchannel))
                cursor = bc_end + TURN_GAP if last else end + BACKCHANNEL_PAUSE
            else:
                cursor = end + (TURN_GAP if last else SENTENCE_PAUSE)
            turn_end = end
        rows.append(row(speaker, turn_start - ROW_PAD, turn_end + ROW_PAD, " ".join(turn_text)))
    frames = round((max(r["end"] for r in rows) + 1.0) * RATE)
    rng = np.random.default_rng(7)
    out = {}
    for speaker, track in tracks.items():
        track = track[:frames] if len(track) >= frames else np.pad(track, (0, frames - len(track)))
        track = track + rng.normal(0.0, 0.0006, frames)  # faint room tone
        out[speaker] = np.clip(np.round(track * 32767), -32768, 32767).astype("<i2")
    rows.sort(key=lambda r: (r["start"], r["speaker"]))
    return out, rows, frames / RATE


def write_wav(path, samples):
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(samples.tobytes())


def ident(name):
    return str(uuid.uuid5(NAMESPACE, name))


def sample_task(rows):
    task_id = ident("task")
    action_id = ident("review-action")
    recordings = {s: ident(f"processed-recording/{s}") for s in VOICES}
    tracks = [
        {
            "track_id": f"track-{speaker}",
            "speaker_key": speaker,
            "path": f"audio/speaker_{speaker[-1]}.wav",
            "processed_recording_id": recordings[speaker],
            "chunked_processed_recording_id": ident(f"chunked-processed-recording/{speaker}"),
            "transcription_chunk_id": task_id,
        }
        for speaker in VOICES
    ]
    annotations = []
    for index, r in enumerate(rows, 1):
        created = f"2026-01-15T10:{index:02d}:00.000Z"
        annotations.append({
            "annotation_id": f"annotation-{index}",
            "babel_original_annotation_id": ident(f"annotation/{index}"),
            "speaker_key": r["speaker"],
            "track_id": f"track-{r['speaker']}",
            "start_seconds": r["start"],
            "end_seconds": r["end"],
            "text": r["text"],
            "reviewActionId": action_id,
            "type": "transcription",
            "processedRecordingId": recordings[r["speaker"]],
            "intensity": None,
            "metadata": {"lowConfidenceResolved": True},
            "createdAt": created,
            "updatedAt": created,
        })
    return {
        "task_id": task_id,
        "project_preset": "babel-review-action",
        "action_id": action_id,
        "action_level": 1,
        "action_worker_id": ident("worker"),
        "action_decision": "in-progress",
        "is_worker_resuming_claim": False,
        "source": "exaudio_demo/synthetic_dialogue",
        "format": "review-action-materialized-package",
        "speakers": [{"speaker_key": s, "label": f"Speaker {s[-1]}"} for s in VOICES],
        "audio_tracks": tracks,
        "annotations": annotations,
    }


def native_project(task):
    labels = {speaker["speaker_key"]: speaker["label"] for speaker in task["speakers"]}
    return {
        "id": "exaudio-synthetic-demo",
        "name": "Synthetic two-speaker demo",
        "speakers": [{"id": key, "label": label} for key, label in labels.items()],
        "lanes": [
            {"id": track["track_id"], "speaker_id": track["speaker_key"],
             "label": labels[track["speaker_key"]], "source_path": Path(track["path"]).name}
            for track in task["audio_tracks"]
        ],
        "segments": [
            {"id": row["annotation_id"], "source_annotation_id": None,
             "speaker_id": row["speaker_key"], "lane_id": row["track_id"],
             "start_seconds": row["start_seconds"], "end_seconds": row["end_seconds"],
             "text": row["text"], "metadata": {}}
            for row in task["annotations"]
        ],
        "metadata": {},
    }


def main():
    tracks, rows, duration = build(sys.argv[1])
    DEMO_DIR.mkdir(parents=True, exist_ok=True)
    for speaker, samples in tracks.items():
        write_wav(DEMO_DIR / f"speaker_{speaker[-1]}.wav", samples)
    task = sample_task(rows)
    SAMPLE_TASK.write_text(json.dumps(task, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    SAMPLE_PROJECT.write_text(json.dumps(native_project(task), ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(f"{len(rows)} rows, {duration:.2f} s")


if __name__ == "__main__":
    main()
