use exaudio_extension::format::{ExportOptions, FormatProvider, FormatRegistry, ImportContent};
use exaudio_formats::{EXTENSION_ID, SubtitleFormat};
use studio_core::{Project, Segment, Speaker};

fn segment(id: &str, start: f64, end: f64, text: &str, speaker: &str) -> Segment {
    Segment {
        id: id.into(),
        source_annotation_id: None,
        speaker_id: speaker.into(),
        lane_id: "audio".into(),
        start_seconds: start,
        end_seconds: end,
        text: text.into(),
        metadata: Default::default(),
    }
}
fn project(segments: Vec<Segment>) -> Project {
    Project {
        id: "synthetic".into(),
        name: "Synthetic transcript".into(),
        speakers: vec![
            Speaker {
                id: "a".into(),
                label: "Alice".into(),
            },
            Speaker {
                id: "b".into(),
                label: "Боб".into(),
            },
        ],
        lanes: vec![],
        segments,
        metadata: Default::default(),
    }
}
fn timed(content: ImportContent) -> Vec<exaudio_extension::format::ImportedCue> {
    match content {
        ImportContent::Timed(cues) => cues,
        ImportContent::Untimed(_) => panic!("expected timed cues"),
    }
}

#[test]
fn known_srt_roundtrip_sorts_stably_and_preserves_overlap_unicode_multiline() {
    let input = project(vec![
        segment("later", 5.0, 7.0, "later", "a"),
        segment("first", 0.0, 2.0, "Привет\n世界", "a"),
        segment("overlap", 0.0, 1.0, "overlap", "b"),
    ]);
    let bytes = SubtitleFormat::Srt
        .export(&input, &ExportOptions::default())
        .unwrap();
    assert_eq!(
        String::from_utf8(bytes.clone()).unwrap(),
        "1\n00:00:00,000 --> 00:00:02,000\nПривет\n世界\n\n2\n00:00:00,000 --> 00:00:01,000\noverlap\n\n3\n00:00:05,000 --> 00:00:07,000\nlater\n\n"
    );
    let cues = timed(SubtitleFormat::Srt.import(&bytes).unwrap());
    assert_eq!(
        cues.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(),
        ["Привет\n世界", "overlap", "later"]
    );
    assert_eq!(cues[0].start_seconds, 0.0);
    assert_eq!(cues[1].end_seconds, 1.0);
}

#[test]
fn vtt_roundtrip_escapes_literal_markup_once() {
    let text = "<script> &amp; & >\n世界 --> result";
    let bytes = SubtitleFormat::WebVtt
        .export(
            &project(vec![segment("a", 1.25, 2.5, text, "a")]),
            &ExportOptions::default(),
        )
        .unwrap();
    let encoded = std::str::from_utf8(&bytes).unwrap();
    assert!(encoded.starts_with("WEBVTT\n\n00:00:01.250 --> 00:00:02.500\n"));
    assert!(encoded.contains("&lt;script&gt; &amp;amp; &amp; &gt;"));
    assert_eq!(
        timed(SubtitleFormat::WebVtt.import(&bytes).unwrap())[0].text,
        text
    );
}

#[test]
fn speaker_labels_are_opt_in_and_escaped() {
    let mut input = project(vec![
        segment("a", 0.0, 1.0, "hello", "a"),
        segment("b", 0.2, 0.8, "да", "b"),
    ]);
    input.speakers[0].label = "<Alice & Co>".into();
    let options = ExportOptions {
        speaker_labels: true,
    };
    for codec in [SubtitleFormat::Srt, SubtitleFormat::WebVtt] {
        let bytes = codec.export(&input, &options).unwrap();
        let cues = timed(codec.import(&bytes).unwrap());
        assert_eq!(cues[0].text, "<Alice & Co>: hello");
        assert_eq!(cues[1].text, "Боб: да");
    }
    assert_eq!(
        SubtitleFormat::Text.export(&input, &options).unwrap(),
        "<Alice & Co>: hello\nБоб: да".as_bytes()
    );
}

#[test]
fn rounds_with_carry_and_supports_long_hours() {
    for (codec, separator) in [(SubtitleFormat::Srt, ','), (SubtitleFormat::WebVtt, '.')] {
        let input = project(vec![
            segment("carry", 3599.9996, 3600.0016, "carry", "a"),
            segment("long", 444_599.999_6, 444_602.0, "long", "a"),
        ]);
        let bytes = codec.export(&input, &ExportOptions::default()).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains(&format!(
            "01:00:00{separator}000 --> 01:00:00{separator}002"
        )));
        assert!(text.contains(&format!("123:30:00{separator}000")));
        let cues = timed(codec.import(&bytes).unwrap());
        assert_eq!(cues[0].start_seconds, 3600.0);
        assert_eq!(cues[1].start_seconds, 444600.0);
    }
}

#[test]
fn bom_crlf_and_common_vtt_settings_markup_and_metadata() {
    let bytes = "\u{feff}WEBVTT demo\r\nLanguage: en\r\n\r\nNOTE ignored\r\ncomment\r\n\r\nSTYLE\r\n::cue { color: red; }\r\n\r\nREGION\r\nid:main\r\n\r\ncue-name\r\n00:01.000 --> 00:03.000 line:90%,end position:50%,center size:80% align:start vertical:rl region:main\r\n<v Alice &amp; Co><b>Hello</b> <c.red>世界</c></v>\r\n<i>next</i><00:02.000>\r\n";
    let cues = timed(SubtitleFormat::WebVtt.import(bytes.as_bytes()).unwrap());
    assert_eq!(cues.len(), 1);
    assert_eq!(cues[0].text, "Hello 世界\nnext");
    assert_eq!(cues[0].speaker.as_deref(), Some("Alice & Co"));
    let srt = timed(
        SubtitleFormat::Srt
            .import("\u{feff}7\r\n00:00:00,000 --> 00:00:01,000\r\n世界\r\n".as_bytes())
            .unwrap(),
    );
    assert_eq!(srt[0].text, "世界");
}

#[test]
fn plain_text_import_never_invents_timing() {
    let bytes = "\u{feff}hello\r\n\r\n世界\r\n";
    assert_eq!(
        SubtitleFormat::Text.import(bytes.as_bytes()).unwrap(),
        ImportContent::Untimed("hello\n\n世界\n".into())
    );
    assert_eq!(
        SubtitleFormat::Text.import(b"").unwrap(),
        ImportContent::Untimed(String::new())
    );
    let input = project(vec![
        segment("b", 2.0, 3.0, "世界", "b"),
        segment("a", 0.0, 1.0, "hello", "a"),
    ]);
    let bytes = SubtitleFormat::Text
        .export(&input, &ExportOptions::default())
        .unwrap();
    assert_eq!(
        SubtitleFormat::Text.import(&bytes).unwrap(),
        ImportContent::Untimed("hello\n世界".into())
    );
}

#[test]
fn invalid_cue_returns_precise_location_without_partial_success() {
    let input =
        b"1\n00:00:00,000 --> 00:00:01,000\nvalid\n\n2\n00:61:00,000 --> 00:62:00,000\nbad\n";
    let error = SubtitleFormat::Srt.import(input).unwrap_err();
    assert_eq!(error.line, Some(6));
    assert_eq!(error.cue, Some(2));
    assert!(error.message.contains("below 60"));
    assert!(error.to_string().contains("line 6"));
}

#[test]
fn rejects_invalid_timings_in_every_export_instead_of_dropping_segments() {
    for codec in [
        SubtitleFormat::Srt,
        SubtitleFormat::WebVtt,
        SubtitleFormat::Text,
    ] {
        for (start, end) in [
            (-1.0, 1.0),
            (0.0, 0.0),
            (2.0, 1.0),
            (f64::NAN, 1.0),
            (0.0, f64::INFINITY),
            (0.0001, 0.0002),
            (0.0, 4.0e9),
        ] {
            let error = codec
                .export(
                    &project(vec![segment("invalid", start, end, "text", "a")]),
                    &ExportOptions::default(),
                )
                .unwrap_err();
            assert_eq!(error.cue, Some(1));
        }
    }
}

#[test]
fn malformed_cues_and_unsupported_markup_are_errors() {
    for input in [
        "WEBVTT\n00:00.000 --> 00:01.000\nmissing header separator",
        "WEBVTT\n\n00:00.000 --> 00:00.000\nzero",
        "WEBVTT\n\n00:00.000 --> 00:01.000 size:101%\nbad settings",
        "WEBVTT\n\n00:00.000 --> 00:01.000 align:left align:right\nduplicate",
        "WEBVTT\n\n00:00.000 --> 00:01.000\n<script>alert</script>",
        "WEBVTT\n\n00:00.000 --> 00:01.000\n<v A>one</v><v B>two</v>",
        "WEBVTT\n\n00:00.000 --> 00:01.000\n&unknown;",
        "WEBVTT\n\n00:00.000 --> 00:01.000\n",
    ] {
        assert!(
            SubtitleFormat::WebVtt.import(input.as_bytes()).is_err(),
            "accepted {input}"
        );
    }
    for input in [
        "1\n-1:00:00,000 --> 00:00:01,000\nnegative",
        "1\n00:00:00,000 --> 00:00:01,00\nprecision",
        "1\n00:00:00,000 --> 00:00:01,000\nfirst\n2\n00:00:02,000 --> 00:00:03,000\nmissing separator",
        "1\n00:00:00,000 --> 00:00:01,000 garbage\nsuffix",
    ] {
        assert!(
            SubtitleFormat::Srt.import(input.as_bytes()).is_err(),
            "accepted {input}"
        );
    }
}

#[test]
fn bounded_utf8_and_blank_subtitle_payload_validation() {
    for codec in [
        SubtitleFormat::Srt,
        SubtitleFormat::WebVtt,
        SubtitleFormat::Text,
    ] {
        assert!(codec.import(&[0xff]).is_err());
        assert!(
            codec
                .import(&vec![b'x'; exaudio_formats::MAX_INPUT_BYTES + 1])
                .is_err()
        );
        assert!(codec.import(b"\0").is_err());
    }
    for codec in [SubtitleFormat::Srt, SubtitleFormat::WebVtt] {
        for text in ["", "hello\n\nworld", "hello\n  \nworld"] {
            assert!(
                codec
                    .export(
                        &project(vec![segment("a", 0.0, 1.0, text, "a")]),
                        &ExportOptions::default()
                    )
                    .is_err()
            );
        }
    }
}

#[test]
fn optional_registry_registration_and_disable_are_real() {
    let mut registry = FormatRegistry::default();
    assert_eq!(registry.entries().len(), 0);
    let manifest = exaudio_formats::manifest();
    manifest.validate().unwrap();
    manifest.check_host_compatibility().unwrap();
    assert_eq!(manifest.id, EXTENSION_ID);
    exaudio_formats::register(&mut registry).unwrap();
    assert_eq!(registry.entries().len(), 3);
    let srt = registry.get("exaudio.formats.srt").unwrap();
    assert_eq!(
        timed(
            srt.import(b"1\n00:00:00,000 --> 00:00:01,000\nhello\n")
                .unwrap()
        )[0]
        .text,
        "hello"
    );
    assert!(exaudio_formats::register(&mut registry).is_err());
    assert_eq!(registry.entries().len(), 3);
    registry.unregister_extension(EXTENSION_ID);
    assert!(registry.entries().is_empty());
    assert!(registry.get("exaudio.formats.srt").is_none());
    exaudio_formats::register(&mut registry).unwrap();
    assert_eq!(registry.entries().len(), 3);
}

#[test]
fn markup_errors_identify_actual_payload_line() {
    let error = SubtitleFormat::WebVtt
        .import(b"WEBVTT\n\n00:00.000 --> 00:01.000\nvalid\n<script>bad</script>\n")
        .unwrap_err();
    assert_eq!(error.cue, Some(1));
    assert_eq!(error.line, Some(5));
}

#[test]
fn srt_literal_arrow_is_text_but_embedded_timing_line_is_rejected() {
    let input = project(vec![segment("a", 0.0, 1.0, "left --> right", "a")]);
    let bytes = SubtitleFormat::Srt
        .export(&input, &ExportOptions::default())
        .unwrap();
    assert_eq!(
        timed(SubtitleFormat::Srt.import(&bytes).unwrap())[0].text,
        "left --> right"
    );
    let input = project(vec![segment(
        "a",
        0.0,
        1.0,
        "00:00:02,000 --> 00:00:03,000",
        "a",
    )]);
    assert!(
        SubtitleFormat::Srt
            .export(&input, &ExportOptions::default())
            .is_err()
    );
}
