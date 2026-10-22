use miclift::resample::Resampler;
use rtrb::RingBuffer;

#[test]
fn sinc_resampler_preserves_tone_at_different_sample_rates() {
    for (input_rate, output_rate) in [
        (48000, 48000),
        (44100, 48000),
        (48000, 44100),
        (96000, 48000),
    ] {
        let (mut tx, mut rx) = RingBuffer::new(200000);
        for index in 0..150000 {
            tx.push(
                0.25 * (std::f32::consts::TAU * 1000.0 * index as f32 / input_rate as f32).sin(),
            )
            .unwrap();
        }
        let mut resampler = Resampler::new(input_rate, output_rate, 100000);
        assert!(resampler.begin_block(&mut rx, 10000));
        let samples: Vec<_> = (0..10000).map(|_| resampler.next(&mut rx)).collect();
        let rms = (samples[100..].iter().map(|x| x * x).sum::<f32>() / 9900.0).sqrt();
        assert!(
            (rms - 0.25 / 2.0_f32.sqrt()).abs() < 0.003,
            "{input_rate} -> {output_rate}: {rms}"
        );
        let crossings = samples[100..]
            .windows(2)
            .filter(|s| s[0] <= 0.0 && s[1] > 0.0)
            .count();
        let frequency = crossings as f32 * output_rate as f32 / 9900.0;
        assert!((frequency - 1000.0).abs() < 10.0);
    }
}
#[test]
fn bounded_underflow_and_backlog_recovery() {
    let (mut tx, mut rx) = RingBuffer::new(4096);
    let mut resampler = Resampler::new(48000, 48000, 128);
    assert!(!resampler.begin_block(&mut rx, 128));
    for _ in 0..3000 {
        tx.push(0.1).unwrap();
    }
    assert!(resampler.begin_block(&mut rx, 128));
    assert!(rx.slots() < 400);
    assert!(resampler.dropped > 0);
    for _ in 0..128 {
        assert!(resampler.next(&mut rx).is_finite());
    }
}

#[test]
fn asynchronous_clocks_do_not_grow_the_queue_without_bound() {
    for drift in [-0.0003, 0.0003] {
        let (mut tx, mut rx) = RingBuffer::new(2048);
        let mut converter = Resampler::new(8000, 8000, 128);
        for _ in 0..256 {
            tx.push(0.01).unwrap();
        }
        let mut fractional = 0.0_f64;
        let mut maximum = 0;
        for _ in 0..12000 {
            fractional += 80.0 * (1.0 + drift);
            let frames = fractional.floor() as usize;
            fractional -= frames as f64;
            for _ in 0..frames {
                tx.push(0.01).unwrap();
            }
            if converter.begin_block(&mut rx, 80) {
                for _ in 0..80 {
                    converter.next(&mut rx);
                }
            }
            maximum = maximum.max(rx.slots());
        }
        assert!(maximum < 512, "drift {drift}: {maximum}");
        assert_eq!(converter.underruns, 0);
        assert_eq!(converter.dropped, 0);
    }
}

#[test]
fn downsampling_rejects_content_above_the_output_nyquist_frequency() {
    let (mut tx, mut rx) = RingBuffer::new(48000);
    for index in 0..40000 {
        tx.push(0.25 * (std::f32::consts::TAU * 30000.0 * index as f32 / 96000.0).sin())
            .unwrap();
    }
    let mut converter = Resampler::new(96000, 48000, 16000);
    assert!(converter.begin_block(&mut rx, 10000));
    let mut energy = 0.0;
    for index in 0..10000 {
        let sample = converter.next(&mut rx);
        if index > 100 {
            energy += sample * sample;
        }
    }
    assert!((energy / 9899.0_f32).sqrt() < 0.01);
}
