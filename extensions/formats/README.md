# exaudio.formats

Optional, bundled native format contributions for SRT, WebVTT and UTF-8 plain text. No Babel dependency, checking rules, audio decoder, native-project serializer, or implicit enablement. `extension.json` declares identity and the required public API version.

## Host integration

The shell owns one `exaudio_extension::format::FormatRegistry` inside its contribution registry. First approve the extension manifest and permissions through the host lifecycle. Only on enabled activation call `exaudio_formats::register(&mut contributions.formats)`. This registers `exaudio.formats.srt`, `exaudio.formats.vtt`, and `exaudio.formats.txt` atomically. Disable/removal must call `unregister_extension("exaudio.formats")`; never keep cached picker entries or providers usable after disable. Build import/export menus from live `entries()` and their capability flags, not a core format list. Registration alone is not permission approval.

`FormatProvider::import(&[u8]) -> Result<ImportContent, FormatError>` produces either:

- `Timed(Vec<ImportedCue>)`: seconds, plain UTF-8 text and optional speaker display label. Attach these records to the user-selected local audio through normal editor transactions. The provider never chooses paths or invents speaker IDs.
- `Untimed(String)`: normalized transcript content with **no fabricated timestamps**. Keep this content in an explicit untimed/manual-timing or alignment workflow. Do not represent it as a zero-duration segment or claim alignment has happened.

`FormatProvider::export(&Project, &ExportOptions) -> Result<Vec<u8>, FormatError>` reads generic editor state. `speaker_labels` defaults to false; when true labels appear as visible `Label: text` prefixes. These prefixes are intentionally not parsed heuristically as speaker metadata on reimport. VTT `<v Speaker>` input does expose an explicit speaker label.

The host owns file picking and file writes, validates imported records against its document/audio constraints, and surfaces provider errors. No partial import result is returned on invalid cues. `FormatError` carries one-based cue and source-line context when known; export cue ordinals refer to the original project order.

## Codec semantics

- Export order is ascending start time, preserving original order for equal starts. Overlaps remain separate cues; there is no speaker merging or clipping.
- Timestamps are rounded once to nearest millisecond before formatting, with full second/minute/hour carry. Start zero is valid. Negative/nonfinite times, nonpositive durations, durations collapsing under rounding, and times beyond `999999:59:59.999` are errors, including TXT export.
- BOM, CRLF and CR input are normalized. UTF-8 is required. Multiline nonblank subtitle text is retained. Empty/blank-line subtitle payloads are rejected on export because a blank line delimits cues. TXT preserves blank lines and returns untimed content.
- SRT accepts numeric cue identifiers or timing-first cues and `HH:MM:SS,mmm`. Text is retained literally; SRT presentation tags are not interpreted. Timing-line suffixes and embedded timing lines are rejected rather than guessed. Ordinary text arrows remain literal.
- VTT accepts a `WEBVTT` header, optional header metadata, cue identifiers, `MM:SS.mmm`/`HH:MM:SS.mmm`, and validated `line`, `position`, `size`, `align`, `vertical`, and `region` settings. NOTE blocks and pre-cue STYLE/REGION blocks are presentation metadata and are not exported again.
- VTT export escapes `&`, `<` and `>` so literal markup cannot become executable/presentation markup. Import decodes the six standard VTT entities, removes supported formatting tags (`b`, `i`, `u`, `c`, `ruby`, `rt`, `lang`), handles voice annotations and inline timestamps, and returns plain text. Styling, karaoke timestamps and layout are intentionally not retained in the generic transcript. Unsupported tags/entities and multiple distinct voices in one cue are explicit errors, not silently lost content. Split multi-voice cues before importing. Literal ampersands/angle brackets must be encoded as entities.
- Limits: 16 MiB per input/output, 100,000 cues, 256 KiB per imported cue payload or complete encoded exported cue, fewer than 1,000,000 newline characters for subtitle input/output. NUL, malformed UTF-8, invalid timestamps/settings, missing payloads and missing cue separators produce errors. These limits are checked before unbounded cue construction; providers perform no I/O or network calls.

## Author example and checks

`examples/roundtrip.rs` uses a synthetic overlapping two-speaker Unicode transcript and roundtrips each provider through the public API:

```text
cargo run -p exaudio-formats --example roundtrip
cargo nextest run -p exaudio-formats -p exaudio-extension
```

Tests exercise known output, UTF-8/multiline text, overlap stability, rounding/carry and long hours, VTT markup safety/settings, honest untimed TXT, error locations, invalid timing and bounded input, plus atomic registration and owner-scoped removal. They do not pin Rust source text.

## Manual UI smoke scenario

1. Enable `exaudio.formats`, open a short local audio file, and create two overlapping segments with different speakers, multiline Unicode text, and a start at zero.
2. Export SRT, VTT and TXT with speaker labels off, then export with labels on. Inspect timestamps and literal text such as `<tag> & 世界` in VTT.
3. Reopen each timed subtitle export against the selected audio. Confirm cue text/timing/overlaps; default SRT speaker labels are not inferred. Reopen TXT and confirm an explicit untimed workflow, not fabricated segments.
4. Import a malformed second cue and confirm a cue/line error with no partial document mutation.
5. Disable the extension and confirm all three import/export choices disappear, while core native project saving and audio editing remain available. Re-enable and confirm entries return once, without duplicates.

The example, nextest suite and UI smoke are validation instructions, not claims they have been run.
