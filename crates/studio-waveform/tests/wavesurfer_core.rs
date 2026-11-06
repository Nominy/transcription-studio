use studio_waveform::{
    DecodedAudio, PeakCache, Region, RegionParams, RegionUpdateSide, RegionsPlugin,
    RendererOptions, WaveSurfer, WaveSurferEvent, WaveSurferOptions, WaveViewport, WaveformHit,
    WaveformHitTarget, WaveformRenderModel, WaveformRenderOptions, WaveformSource,
    decode_pcm16_wav, export_peaks, export_range_peaks,
};

#[test]
fn demo_speaker_wavs_decode_and_export_waveform_peaks() {
    let demo_dir =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../studio-editor/assets/demo");

    for name in ["speaker_1.wav", "speaker_2.wav"] {
        let bytes = std::fs::read(demo_dir.join(name)).expect("demo speaker wav");
        let audio = decode_pcm16_wav(&bytes).expect("demo speaker wav should decode");

        assert_eq!(audio.sample_rate(), 44_100);
        assert_eq!(audio.number_of_channels(), 1);
        assert_eq!(audio.frame_count(), 5_133_240);
        assert!((audio.duration() - 116.4).abs() < 0.000_01);

        let peaks = export_peaks(&audio, 1, 128, 100.0).expect("demo wav peaks");
        assert_eq!(peaks.len(), 1);
        assert_eq!(peaks[0].len(), 128);
        assert!(
            peaks[0].iter().any(|peak| peak.abs() > 0.01),
            "{name} should not decode as silence"
        );
    }
}

#[test]
fn export_peaks_matches_wavesurfer_bucket_max_with_signed_peak_and_precision() {
    let audio = DecodedAudio::new(
        4,
        vec![vec![0.10, -0.50, 0.25, 0.75, -0.90, 0.20, 0.40, -0.30]],
    )
    .expect("valid mono audio");

    let peaks = export_peaks(&audio, 2, 4, 100.0).expect("peaks should export");

    assert_eq!(peaks, vec![vec![-0.5, 0.75, -0.9, 0.4]]);
}

#[test]
fn export_peaks_limits_channels_and_rounds_each_bucket_like_wavesurfer() {
    let audio = DecodedAudio::new(
        4,
        vec![
            vec![0.1234, -0.4567, 0.3333, 0.2222],
            vec![-0.1111, 0.2222, -0.8888, 0.4444],
            vec![0.9999, 0.1111, 0.2222, 0.3333],
        ],
    )
    .expect("valid multi-channel audio");

    let peaks = export_peaks(&audio, 2, 2, 10.0).expect("peaks should export");

    assert_eq!(peaks, vec![vec![-0.5, 0.3], vec![0.2, -0.9]]);
}

#[test]
fn export_range_peaks_uses_audio_slice_before_bucket_max() {
    let audio = DecodedAudio::new(
        4,
        vec![vec![
            0.10, -0.20, 0.30, -0.40, //
            0.50, -0.90, 0.20, -0.10,
        ]],
    )
    .expect("valid mono audio");

    let peaks =
        export_range_peaks(&audio, 0.25, 1.5, 1, 2, 100.0).expect("range peaks should export");

    assert_eq!(peaks, vec![vec![-0.4, -0.9]]);
}

#[test]
fn renderer_layout_matches_wavesurfer_min_px_per_second_and_fill_parent_rules() {
    let audio = DecodedAudio::new(10, vec![vec![0.0; 20]]).expect("valid audio");
    let wave = WaveSurfer::from_decoded(
        audio,
        WaveSurferOptions {
            min_px_per_sec: 200.0,
            fill_parent: false,
            ..WaveSurferOptions::default()
        },
        RendererOptions {
            parent_width_px: 350.0,
            pixel_ratio: 1.0,
        },
    );

    let layout = wave.layout();

    assert_eq!(wave.get_duration(), 2.0);
    assert_eq!(layout.scroll_width_px, 400.0);
    assert_eq!(layout.wrapper_width_px, 400.0);
    assert!(layout.is_scrollable);
    assert!(!layout.uses_parent_width);
    assert_eq!(wave.get_width(), 350.0);
}

#[test]
fn zoom_updates_min_px_per_second_and_preserves_wavesurfer_scroll_contract() {
    let audio = DecodedAudio::new(10, vec![vec![0.0; 20]]).expect("valid audio");
    let mut wave = WaveSurfer::from_decoded(
        audio,
        WaveSurferOptions {
            min_px_per_sec: 0.0,
            fill_parent: true,
            ..WaveSurferOptions::default()
        },
        RendererOptions {
            parent_width_px: 500.0,
            pixel_ratio: 1.0,
        },
    );

    assert_eq!(wave.layout().wrapper_width_px, 500.0);
    assert_eq!(wave.get_scroll(), 0.0);

    wave.zoom(300.0);
    wave.set_scroll(125.0);

    assert_eq!(wave.options().min_px_per_sec, 300.0);
    assert_eq!(wave.layout().scroll_width_px, 600.0);
    assert_eq!(wave.layout().wrapper_width_px, 600.0);
    assert_eq!(wave.get_scroll(), 100.0);
}

#[test]
fn progress_geometry_uses_percent_left_like_wavesurfer_renderer() {
    let audio = DecodedAudio::new(10, vec![vec![0.0; 40]]).expect("valid audio");
    let wave = WaveSurfer::from_decoded(
        audio,
        WaveSurferOptions::default(),
        RendererOptions {
            parent_width_px: 400.0,
            pixel_ratio: 1.0,
        },
    );

    let progress = wave.progress_geometry(1.25, 2.0);

    assert_eq!(progress.progress_percent, 31.25);
    assert_eq!(progress.cursor_left_percent, 31.25);
    assert_eq!(progress.cursor_translate_x_px, -0.625);
}

#[test]
fn time_pixel_mapping_accounts_for_zoom_width_and_scroll_like_wavesurfer() {
    let audio = DecodedAudio::new(10, vec![vec![0.0; 40]]).expect("valid audio");
    let mut wave = WaveSurfer::from_decoded(
        audio,
        WaveSurferOptions {
            min_px_per_sec: 200.0,
            fill_parent: false,
            ..WaveSurferOptions::default()
        },
        RendererOptions {
            parent_width_px: 300.0,
            pixel_ratio: 1.0,
        },
    );

    wave.set_scroll(150.0);

    assert_eq!(wave.seconds_to_content_px(1.25), 250.0);
    assert_eq!(wave.seconds_to_viewport_px(1.25), 100.0);
    assert_eq!(wave.viewport_px_to_seconds(100.0), 1.25);
    assert_eq!(wave.viewport_px_to_seconds(-500.0), 0.0);
    assert_eq!(wave.viewport_px_to_seconds(10_000.0), 4.0);
}

#[test]
fn interaction_toggle_seek_and_scroll_time_follow_wavesurfer_events() {
    let audio = DecodedAudio::new(10, vec![vec![0.0; 40]]).expect("valid audio");
    let mut wave = WaveSurfer::from_decoded(
        audio,
        WaveSurferOptions {
            min_px_per_sec: 200.0,
            fill_parent: false,
            ..WaveSurferOptions::default()
        },
        RendererOptions {
            parent_width_px: 300.0,
            pixel_ratio: 1.0,
        },
    );

    wave.toggle_interaction(false);
    assert!(wave.click(0.25, 0.75).is_empty());
    assert_eq!(wave.get_current_time(), 0.0);

    wave.toggle_interaction(true);
    let events = wave.click(0.25, 0.75);

    assert_eq!(wave.get_current_time(), 1.0);
    assert_eq!(
        events,
        vec![
            WaveSurferEvent::TimeUpdate(1.0),
            WaveSurferEvent::Interaction(1.0),
            WaveSurferEvent::Click {
                relative_x: 0.25,
                relative_y: 0.75,
            },
        ]
    );

    wave.seek_to(0.5);
    assert_eq!(wave.get_current_time(), 2.0);

    let scroll = wave.set_scroll_time(1.25);
    assert_eq!(wave.get_scroll(), 250.0);
    assert_eq!(scroll.visible_start_seconds, 1.25);
    assert_eq!(scroll.visible_end_seconds, 2.75);
    assert_eq!(scroll.scroll_left_px, 250.0);
    assert_eq!(scroll.scroll_right_px, 550.0);
}

#[test]
fn update_progress_auto_centers_out_of_view_cursor_like_wavesurfer() {
    let audio = DecodedAudio::new(10, vec![vec![0.0; 40]]).expect("valid audio");
    let mut wave = WaveSurfer::from_decoded(
        audio,
        WaveSurferOptions {
            min_px_per_sec: 200.0,
            fill_parent: false,
            auto_scroll: true,
            auto_center: true,
            ..WaveSurferOptions::default()
        },
        RendererOptions {
            parent_width_px: 300.0,
            pixel_ratio: 1.0,
        },
    );

    let events = wave.update_progress(2.0, false);

    assert_eq!(wave.get_current_time(), 2.0);
    assert_eq!(wave.get_scroll(), 250.0);
    assert_eq!(
        events,
        vec![
            WaveSurferEvent::TimeUpdate(2.0),
            WaveSurferEvent::Scroll(wave.scroll_window()),
        ]
    );
    assert_eq!(wave.scroll_window().visible_start_seconds, 1.25);
    assert_eq!(wave.scroll_window().visible_end_seconds, 2.75);
}

#[test]
fn update_progress_scrolls_during_playback_after_center_like_wavesurfer() {
    let audio = DecodedAudio::new(10, vec![vec![0.0; 40]]).expect("valid audio");
    let mut wave = WaveSurfer::from_decoded(
        audio,
        WaveSurferOptions {
            min_px_per_sec: 200.0,
            fill_parent: false,
            auto_scroll: true,
            auto_center: true,
            ..WaveSurferOptions::default()
        },
        RendererOptions {
            parent_width_px: 300.0,
            pixel_ratio: 1.0,
        },
    );

    let events = wave.update_progress(0.8, true);

    assert_eq!(wave.get_current_time(), 0.8);
    assert_eq!(wave.get_scroll(), 10.0);
    assert_eq!(
        events,
        vec![
            WaveSurferEvent::TimeUpdate(0.8),
            WaveSurferEvent::Scroll(wave.scroll_window()),
        ]
    );
    assert!((wave.scroll_window().visible_start_seconds - 0.05).abs() < 0.000_001);
    assert!((wave.scroll_window().visible_end_seconds - 1.55).abs() < 0.000_001);
}

#[test]
fn update_progress_respects_disabled_auto_scroll_option() {
    let audio = DecodedAudio::new(10, vec![vec![0.0; 40]]).expect("valid audio");
    let mut wave = WaveSurfer::from_decoded(
        audio,
        WaveSurferOptions {
            min_px_per_sec: 200.0,
            fill_parent: false,
            auto_scroll: false,
            auto_center: true,
            ..WaveSurferOptions::default()
        },
        RendererOptions {
            parent_width_px: 300.0,
            pixel_ratio: 1.0,
        },
    );

    let events = wave.update_progress(2.0, true);

    assert_eq!(wave.get_current_time(), 2.0);
    assert_eq!(wave.get_scroll(), 0.0);
    assert_eq!(events, vec![WaveSurferEvent::TimeUpdate(2.0)]);
}

#[test]
fn wavesurfer_resize_parent_width_clamps_scroll_and_emits_resize() {
    let audio = DecodedAudio::new(10, vec![vec![0.0; 40]]).expect("valid audio");
    let mut wave = WaveSurfer::from_decoded(
        audio,
        WaveSurferOptions {
            min_px_per_sec: 200.0,
            fill_parent: false,
            ..WaveSurferOptions::default()
        },
        RendererOptions {
            parent_width_px: 300.0,
            pixel_ratio: 1.0,
        },
    );
    wave.set_scroll(500.0);

    let event = wave.resize_parent_width(600.0);

    assert_eq!(event, WaveSurferEvent::Resize(600.0));
    assert_eq!(wave.get_width(), 600.0);
    assert_eq!(wave.get_scroll(), 200.0);
    assert_eq!(wave.scroll_window().visible_start_seconds, 1.0);
}

#[test]
fn render_window_samples_slice_decoded_audio_like_wavesurfer_renderer() {
    let audio = DecodedAudio::new(1, vec![vec![0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7]])
        .expect("valid audio");
    let wave = WaveSurfer::from_decoded(
        audio,
        WaveSurferOptions {
            min_px_per_sec: 100.0,
            fill_parent: false,
            ..WaveSurferOptions::default()
        },
        RendererOptions {
            parent_width_px: 250.0,
            pixel_ratio: 1.0,
        },
    );

    let channels = wave.render_window_samples(200.0, 300.0, 1);

    assert_eq!(channels, vec![vec![0.2, 0.3, 0.4]]);
}

#[test]
fn region_virtual_visibility_matches_wavesurfer_plugin_window_rules() {
    let mut regions = RegionsPlugin::new(10.0, 1).unwrap();
    regions
        .add_region(RegionParams::new("visible", 2.0, Some(4.0)))
        .unwrap();
    regions
        .add_region(RegionParams::new("hidden", 8.0, Some(9.0)))
        .unwrap();
    regions
        .add_region(RegionParams::new("marker", 4.5, Some(4.5)))
        .unwrap();

    let visible = regions
        .virtual_visibility("visible", 250.0, 200.0, 1_000.0)
        .unwrap();
    let hidden = regions
        .virtual_visibility("hidden", 250.0, 200.0, 1_000.0)
        .unwrap();
    let marker = regions
        .virtual_visibility("marker", 449.0, 2.0, 1_000.0)
        .unwrap();

    assert_eq!(visible.start_px, 200.0);
    assert_eq!(visible.end_px, 400.0);
    assert!(visible.is_visible);
    assert!(!hidden.is_visible);
    assert_eq!(marker.end_px - marker.start_px, 1.0);
    assert!(marker.is_visible);
}

#[test]
fn region_scroll_adjustment_matches_wavesurfer_plugin_adjust_scroll() {
    let mut regions = RegionsPlugin::new(10.0, 1).unwrap();
    regions
        .add_region(RegionParams::new("right", 8.0, Some(9.0)))
        .unwrap();
    regions
        .add_region(RegionParams::new("left", 1.0, Some(2.0)))
        .unwrap();

    assert_eq!(
        regions.adjust_scroll_for_region("right", 0.0, 200.0, 1_000.0),
        Some(700.0)
    );
    assert_eq!(
        regions.adjust_scroll_for_region("left", 300.0, 200.0, 1_000.0),
        Some(100.0)
    );
    assert_eq!(
        regions.adjust_scroll_for_region("left", 100.0, 200.0, 1_000.0),
        Some(100.0)
    );
}

#[test]
fn precomputed_peak_source_exports_without_decoded_audio() {
    let source =
        WaveformSource::from_precomputed_peaks(vec![vec![0.0, 0.5, -0.75, 0.25, 1.0, -0.25]], 3.0)
            .expect("valid peaks");

    assert_eq!(source.duration(), 3.0);
    assert_eq!(source.number_of_channels(), 1);
    assert_eq!(
        source.export_peaks(1, 3, 100.0).unwrap(),
        vec![vec![0.5, -0.75, 1.0]]
    );
    assert_eq!(
        source.render_window_samples(2.0, 2.0, 6.0, 1),
        vec![vec![-0.75, 0.25]]
    );
}

#[test]
fn peak_cache_reuses_resolution_and_can_render_visible_ranges() {
    let audio = DecodedAudio::new(4, vec![vec![0.0, 0.25, -0.5, 0.75, -1.0, 0.5, 0.25, 0.0]])
        .expect("valid audio");
    let mut cache = PeakCache::new(WaveformSource::from_decoded(audio));

    let first = cache.peaks_for_resolution(4, 100.0).unwrap();
    let second = cache.peaks_for_resolution(4, 100.0).unwrap();

    assert_eq!(first, second);
    assert_eq!(cache.cached_resolution_count(), 1);
    assert_eq!(
        cache.visible_peaks(1.0, 2.0, 1, 4, 100.0).unwrap(),
        vec![vec![-1.0, 0.5, 0.25, 0.0]]
    );
}

#[test]
fn visible_peak_cache_slices_borrowed_resolution_without_full_clone() {
    let cache_source = include_str!("../src/cache.rs");

    assert!(cache_source.contains("fn cached_peaks_for_resolution("));
    assert!(
        cache_source.contains("self.cached_peaks_for_resolution(resolution_length, precision)?")
    );
    assert!(!cache_source.contains("let full_resolution = self.peaks_for_resolution"));
}

#[test]
fn render_model_reuses_cached_peak_resolution_for_moving_viewports() {
    let source = WaveformSource::from_decoded(
        DecodedAudio::new(
            10,
            vec![
                (0..100)
                    .map(|index| (index as f64 / 100.0 * 2.0) - 1.0)
                    .collect(),
            ],
        )
        .expect("valid audio"),
    );
    let mut cache = PeakCache::new(source);
    let options = WaveformRenderOptions {
        channels: 1,
        peak_count: 8,
        precision: 100.0,
        ..WaveformRenderOptions::default()
    };

    let mut first_viewport = WaveViewport::new(10.0, 200.0, 100.0, false, 1.0);
    first_viewport.set_scroll_time(1.0);
    let first = WaveformRenderModel::from_peak_cache(&mut cache, first_viewport, &[], 1.5, options)
        .expect("first render model should build");

    let mut second_viewport = WaveViewport::new(10.0, 200.0, 100.0, false, 1.0);
    second_viewport.set_scroll_time(2.0);
    let second =
        WaveformRenderModel::from_peak_cache(&mut cache, second_viewport, &[], 2.5, options)
            .expect("second render model should build");

    assert_eq!(cache.cached_resolution_count(), 1);
    assert_eq!(first.visible_start_seconds, 1.0);
    assert_eq!(second.visible_start_seconds, 2.0);
    assert_eq!(first.channels[0].peaks.len(), 8);
    assert_eq!(second.channels[0].peaks.len(), 8);
}

#[test]
fn viewport_zoom_at_keeps_anchor_time_under_pointer() {
    let mut viewport = WaveViewport::new(10.0, 500.0, 100.0, false, 1.0);
    viewport.set_scroll(250.0);

    let anchor_time = viewport.viewport_px_to_seconds(100.0);
    viewport.zoom_at(100.0, 200.0);

    assert_eq!(anchor_time, 3.5);
    assert_eq!(viewport.min_px_per_sec(), 200.0);
    assert_eq!(viewport.get_scroll(), 600.0);
    assert_eq!(viewport.viewport_px_to_seconds(100.0), anchor_time);
}

#[test]
fn render_model_clips_peaks_regions_cursor_to_visible_viewport() {
    let source = WaveformSource::from_decoded(
        DecodedAudio::new(2, vec![vec![0.0, 0.25, -0.5, 0.75, -1.0, 0.5, 0.25, 0.0]])
            .expect("valid audio"),
    );
    let mut viewport = WaveViewport::new(4.0, 200.0, 100.0, false, 1.0);
    viewport.set_scroll_time(1.0);
    let mut regions = RegionsPlugin::new(4.0, 2).unwrap();
    regions
        .add_region(RegionParams::new("left-edge", 0.5, Some(1.5)).with_channel_index(0))
        .unwrap();
    regions
        .add_region(RegionParams::new("right-edge", 2.5, Some(3.5)).with_channel_index(1))
        .unwrap();

    let model = WaveformRenderModel::from_source(
        &source,
        viewport,
        regions.get_regions(),
        2.0,
        WaveformRenderOptions {
            channels: 1,
            peak_count: 4,
            precision: 100.0,
            cursor_width_px: 2.0,
            handle_width_px: 6.0,
        },
    )
    .expect("render model should build");

    assert_eq!(model.visible_start_seconds, 1.0);
    assert_eq!(model.visible_end_seconds, 3.0);
    assert_eq!(model.channels[0].peaks.as_ref(), &[-0.5, 0.75, -1.0, 0.5]);
    assert_eq!(model.cursor.progress_percent, 50.0);
    assert_eq!(model.regions.len(), 2);
    assert_eq!(model.regions[0].id, "left-edge");
    assert_eq!(model.regions[0].left_percent, 0.0);
    assert_eq!(model.regions[0].right_percent, 75.0);
    assert_eq!(model.regions[1].id, "right-edge");
    assert_eq!(model.regions[1].left_percent, 75.0);
    assert_eq!(model.regions[1].right_percent, 0.0);
    assert_eq!(model.regions[1].top_percent, 50.0);
}

#[test]
fn render_model_hit_test_prefers_region_handles_then_body_then_waveform() {
    let source = WaveformSource::from_precomputed_peaks(vec![vec![0.0; 8]], 4.0).unwrap();
    let viewport = WaveViewport::new(4.0, 400.0, 0.0, true, 1.0);
    let mut regions = RegionsPlugin::new(4.0, 1).unwrap();
    regions
        .add_region(RegionParams::new("annotation-1", 1.0, Some(3.0)))
        .unwrap();
    let model = WaveformRenderModel::from_source(
        &source,
        viewport,
        regions.get_regions(),
        0.0,
        WaveformRenderOptions::default(),
    )
    .unwrap();

    assert_eq!(
        model.hit_test(101.0, 25.0),
        WaveformHit {
            target: WaveformHitTarget::RegionStartHandle,
            region_id: Some("annotation-1".to_string()),
            time_seconds: 1.01,
        }
    );
    assert_eq!(
        model.hit_test(295.0, 25.0).target,
        WaveformHitTarget::RegionEndHandle
    );
    assert_eq!(
        model.hit_test(200.0, 25.0).target,
        WaveformHitTarget::RegionBody
    );
    assert_eq!(
        model.hit_test(20.0, 25.0),
        WaveformHit {
            target: WaveformHitTarget::Waveform,
            region_id: None,
            time_seconds: 0.2,
        }
    );
}

#[test]
fn region_geometry_matches_wavesurfer_region_left_and_right_percentages() {
    let region = Region::new("annotation-7", 2.0, 5.0, 10.0).expect("valid region");

    let geometry = region.geometry();

    assert_eq!(geometry.left_percent, 20.0);
    assert_eq!(geometry.right_percent, 50.0);
    assert!(!geometry.is_marker);
}

#[test]
fn regions_plugin_add_update_remove_follow_wavesurfer_region_rules() {
    let mut regions = RegionsPlugin::new(10.0, 2).expect("valid regions plugin");
    let params = RegionParams::new("annotation-7", 2.0, Some(4.0))
        .with_min_length(0.5)
        .with_max_length(5.0)
        .with_channel_index(1);

    let region = regions.add_region(params).expect("region should be added");

    assert_eq!(region.id, "annotation-7");
    assert_eq!(region.start_seconds, 2.0);
    assert_eq!(region.end_seconds, 4.0);
    assert_eq!(regions.get_regions().len(), 1);
    let geometry = region.geometry();
    assert_eq!(geometry.left_percent, 20.0);
    assert_eq!(geometry.right_percent, 60.0);
    assert_eq!(geometry.top_percent, 50.0);
    assert_eq!(geometry.height_percent, 50.0);

    regions
        .update_region_by_pixels(
            "annotation-7",
            100.0,
            1_000.0,
            Some(RegionUpdateSide::End),
            None,
        )
        .expect("region end resize should apply");
    let region = regions.region("annotation-7").unwrap();
    assert_eq!(region.start_seconds, 2.0);
    assert_eq!(region.end_seconds, 5.0);

    regions
        .update_region_by_pixels("annotation-7", -300.0, 1_000.0, None, None)
        .expect("region move should apply");
    let region = regions.region("annotation-7").unwrap();
    assert_eq!(region.start_seconds, 0.0);
    assert_eq!(region.end_seconds, 2.0);

    let removed = regions
        .remove_region("annotation-7")
        .expect("region should be removed");
    assert_eq!(removed.id, "annotation-7");
    assert!(regions.get_regions().is_empty());
}

#[test]
fn decodes_pcm16_wav_into_deinterleaved_decoded_audio_channels() {
    let bytes = pcm16_wav_bytes(
        8_000,
        2,
        &[
            -32768, 32767, //
            0, 16384, //
            8192, -8192,
        ],
    );

    let audio = decode_pcm16_wav(&bytes).expect("wav should decode");

    assert_eq!(audio.sample_rate(), 8_000);
    assert_eq!(audio.number_of_channels(), 2);
    assert_eq!(audio.frame_count(), 3);
    assert_eq!(audio.duration(), 3.0 / 8_000.0);
    assert_eq!(audio.channel_data(0).unwrap(), &[-1.0, 0.0, 0.25]);
    assert_eq!(
        audio.channel_data(1).unwrap(),
        &[32767.0 / 32768.0, 0.5, -0.25]
    );
}

fn pcm16_wav_bytes(sample_rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
    let data_size = samples.len() * 2;
    let mut bytes = Vec::with_capacity(44 + data_size);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_size as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * channels as u32 * 2).to_le_bytes());
    bytes.extend_from_slice(&(channels * 2).to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data_size as u32).to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}
