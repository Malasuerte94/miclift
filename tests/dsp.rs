use miclift::{
    config::{Config, VoiceSettings},
    dsp::{Processor, response_db},
    shared::Parameters,
};

fn settled(settings: VoiceSettings) -> Processor {
    let mut processor = Processor::new(48000.0);
    processor.configure(&settings);
    for _ in 0..48000 {
        processor.process(0.0);
    }
    processor
}
#[test]
fn unity_is_transparent_and_five_times_below_limiter() {
    for gain in [0.0, 100.0, 250.0, 500.0] {
        let mut p = settled(VoiceSettings {
            boost: gain,
            ..Default::default()
        });
        for index in 0..48000 {
            let sample = 0.1 * (index as f32 * 0.13).sin();
            assert!((p.process(sample) - sample * gain / 100.0).abs() < 0.00002);
        }
    }
}
#[test]
fn limiter_is_bounded_and_nonfinite_input_is_safe() {
    let mut p = settled(VoiceSettings {
        boost: 500.0,
        ..Default::default()
    });
    for input in [1.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.5] {
        let value = p.process(input);
        assert!(value.is_finite() && value.abs() <= 0.981);
    }
}
#[test]
fn eq_center_gain_and_low_cut_match_response() {
    let mut settings = VoiceSettings::default();
    settings.bands[2].gain_db = 6.0;
    assert!((response_db(&settings, 48000.0, 1000.0) - 6.0).abs() < 0.02);
    let mut p = settled(settings);
    let (mut before, mut after) = (0.0, 0.0);
    for i in 0..48000 {
        let input = 0.05 * (i as f32 * std::f32::consts::TAU * 1000.0 / 48000.0).sin();
        let output = p.process(input);
        if i > 24000 {
            before += input * input;
            after += output * output;
        }
    }
    assert!((10.0_f32 * (after / before).log10() - 6.0).abs() < 0.03);
    settings.high_pass = true;
    assert!(response_db(&settings, 48000.0, 20.0) < -20.0);
}
#[test]
fn gate_closes_and_compressor_reduces_loud_signal() {
    let mut p = settled(VoiceSettings {
        gate: true,
        ..Default::default()
    });
    for _ in 0..48000 {
        p.process(0.0001);
    }
    assert!(p.process(0.0001).abs() < 1.0e-8);
    let mut p = settled(VoiceSettings {
        compressor: true,
        limiter: false,
        ..Default::default()
    });
    for _ in 0..48000 {
        p.process(0.5);
    }
    assert!(p.process(0.5) < 0.25);
}
#[test]
fn mute_and_gain_changes_are_smoothed() {
    let mut p = settled(VoiceSettings::default());
    for _ in 0..1000 {
        p.process(0.1);
    }
    p.set_muted(true);
    let first = p.process(0.1);
    assert!(first > 0.09 && first < 0.1);
    for _ in 0..48000 {
        p.process(0.1);
    }
    assert!(p.process(0.1).abs() < 1.0e-6);
}
#[test]
fn profiles_use_endpoint_ids_and_untrusted_values_are_sanitized() {
    let mut config = Config {
        input_id: "endpoint-a".into(),
        ..Default::default()
    };
    config.voice.boost = 350.0;
    config.select_input("endpoint-b".into());
    assert_eq!(config.voice.boost, 100.0);
    config.select_input("endpoint-a".into());
    assert_eq!(config.voice.boost, 350.0);
    config.voice.boost = f32::NAN;
    config.voice.bands[0].q = 0.0;
    config.voice.sanitize();
    assert_eq!(config.voice.boost, 100.0);
    assert!(config.voice.bands[0].q > 0.0);
}
#[test]
fn atomic_settings_are_consistent() {
    let p = Parameters::default();
    let mut sequence = 0;
    let voice = p.load_changed(&mut sequence).unwrap();
    assert_eq!(voice, VoiceSettings::default());
    assert!(p.load_changed(&mut sequence).is_none());
    let changed = VoiceSettings {
        boost: 500.0,
        gate: true,
        ..Default::default()
    };
    p.store(&changed);
    assert_eq!(p.load_changed(&mut sequence).unwrap(), changed);
}
