#[cfg(windows)]
pub mod audio;
pub mod config;
pub mod dsp;
#[cfg(windows)]
pub mod platform;
pub mod resample;
pub mod shared;

pub fn self_test() -> anyhow::Result<()> {
    let mut dsp = dsp::Processor::new(48_000.0);
    let settings = config::VoiceSettings {
        boost: 500.0,
        ..Default::default()
    };
    dsp.configure(&settings);
    for _ in 0..48_000 {
        dsp.process(0.0);
    }
    let output = dsp.process(0.1);
    anyhow::ensure!((output - 0.5).abs() < 0.001, "5x DSP gain self-test failed");
    for _ in 0..48_000 {
        let output = dsp.process(1.0);
        anyhow::ensure!(
            output.is_finite() && output.abs() <= 1.0,
            "limiter self-test failed"
        );
    }
    Ok(())
}
