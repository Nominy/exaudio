use exaudio_waveform::{
    DecodedAudio, PeakCache, WaveViewport, WaveformRenderModel, WaveformRenderOptions,
    WaveformSource, decode_pcm16_wav, export_peaks, export_range_peaks,
};

#[test]
fn demo_speaker_wavs_decode_and_export_waveform_peaks() {
    let demo_dir =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../exaudio-editor/assets/demo");

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
fn viewport_layout_follows_min_px_per_second_and_fill_parent_rules() {
    let zoomed = WaveViewport::new(2.0, 350.0, 200.0, false, 1.0).layout();

    assert_eq!(zoomed.scroll_width_px, 400.0);
    assert_eq!(zoomed.wrapper_width_px, 400.0);
    assert!(zoomed.is_scrollable);
    assert!(!zoomed.uses_parent_width);

    let filled = WaveViewport::new(2.0, 500.0, 0.0, true, 1.0).layout();

    assert_eq!(filled.scroll_width_px, 0.0);
    assert_eq!(filled.wrapper_width_px, 500.0);
    assert!(!filled.is_scrollable);
    assert!(filled.uses_parent_width);
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
}

#[test]
fn render_model_reuses_cached_peak_resolution_across_renders() {
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
    let viewport = WaveViewport::new(10.0, 200.0, 100.0, false, 1.0);
    let options = WaveformRenderOptions {
        channels: 1,
        peak_count: 8,
        precision: 100.0,
        ..WaveformRenderOptions::default()
    };

    let first = WaveformRenderModel::from_peak_cache(&mut cache, viewport, 0.5, options)
        .expect("first render model should build");
    let cache_after_first = cache.clone();
    let second = WaveformRenderModel::from_peak_cache(&mut cache, viewport, 1.5, options)
        .expect("second render model should build");

    assert_eq!(cache, cache_after_first);
    assert_eq!(first.visible_end_seconds, 2.0);
    assert_eq!(first.channels[0].peaks.len(), 8);
    assert_eq!(second.channels, first.channels);
}

#[test]
fn render_model_clips_peaks_and_cursor_to_visible_viewport() {
    let source = WaveformSource::from_decoded(
        DecodedAudio::new(2, vec![vec![0.0, 0.25, -0.5, 0.75, -1.0, 0.5, 0.25, 0.0]])
            .expect("valid audio"),
    );
    let mut cache = PeakCache::new(source);
    let viewport = WaveViewport::new(4.0, 4.0, 2.0, false, 1.0);

    let model = WaveformRenderModel::from_peak_cache(
        &mut cache,
        viewport,
        1.0,
        WaveformRenderOptions {
            channels: 1,
            peak_count: 4,
            precision: 100.0,
            cursor_width_px: 2.0,
        },
    )
    .expect("render model should build");

    assert_eq!(model.visible_start_seconds, 0.0);
    assert_eq!(model.visible_end_seconds, 2.0);
    assert_eq!(model.channels[0].peaks.as_ref(), &[0.0, 0.25, -0.5, 0.75]);
    assert_eq!(model.cursor.progress_percent, 50.0);
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
