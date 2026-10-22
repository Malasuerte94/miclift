use miclift::{config::VoiceSettings, dsp::Processor, resample::Resampler, shared::Parameters};
use rtrb::RingBuffer;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

struct CheckedAllocator;
thread_local! {
    static TRACK: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}
// This wrapper only observes allocations on the current test thread and delegates
// the original layout/pointer unchanged to Rust's system allocator.
unsafe impl GlobalAlloc for CheckedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = TRACK.try_with(|track| {
            if track.get() {
                ALLOCATIONS.with(|count| count.set(count.get() + 1));
            }
        });
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe {
            System.dealloc(pointer, layout);
        }
    }
}
#[global_allocator]
static ALLOCATOR: CheckedAllocator = CheckedAllocator;

#[test]
fn healthy_audio_processing_and_parameter_changes_allocate_nothing() {
    let parameters = Parameters::default();
    let mut processor = Processor::new(48000.0);
    let mut resampler = Resampler::new(48000, 48000, 128);
    let (mut tx, mut rx) = RingBuffer::new(2048);
    for _ in 0..256 {
        tx.push(0.01).unwrap();
    }
    let mut sequence = 0;
    let settings = VoiceSettings {
        boost: 500.0,
        compressor: true,
        gate: true,
        high_pass: true,
        ..Default::default()
    };
    ALLOCATIONS.with(|count| count.set(0));
    TRACK.with(|track| track.set(true));
    for block in 0..1000 {
        if block % 100 == 0 {
            parameters.store(&settings);
        }
        if let Some(settings) = parameters.load_changed(&mut sequence) {
            processor.configure(&settings);
        }
        for _ in 0..64 {
            tx.push(processor.process(0.01)).unwrap();
        }
        if resampler.begin_block(&mut rx, 64) {
            for _ in 0..64 {
                std::hint::black_box(resampler.next(&mut rx));
            }
        }
    }
    TRACK.with(|track| track.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
}
