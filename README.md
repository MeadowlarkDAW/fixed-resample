# fixed-resample

[![Documentation](https://docs.rs/fixed-resample/badge.svg)](https://docs.rs/fixed-resample)
[![Crates.io](https://img.shields.io/crates/v/fixed-resample.svg)](https://crates.io/crates/fixed-resample)
[![License](https://img.shields.io/crates/l/fixed-resample.svg)](https://github.com/MeadowlarkDAW/fixed-resample/blob/main/LICENSE)

An easy to use crate for resampling at a fixed ratio.

It supports resampling in both realtime and in non-realtime applications, and also includes a handy spsc ring buffer type that automatically resamples the input stream to match the output stream when needed.

This crate uses [Rubato](https://github.com/henquist/rubato) internally.

## Non-realtime example

```rust
const IN_SAMPLE_RATE: u32 = 44100;
const OUT_SAMPLE_RATE: u32 = 48000;
const LEN_SECONDS: f64 = 1.0;

// Generate a sine wave at the input sample rate.
let mut phasor: f32 = 0.0;
let phasor_inc: f32 = 440.0 / IN_SAMPLE_RATE as f32;
let len_samples = (LEN_SECONDS * IN_SAMPLE_RATE as f64).round() as usize;
let in_samples: Vec<f32> = (0..len_samples).map(|_| {
    phasor = (phasor + phasor_inc).fract();
    (phasor * std::f32::consts::TAU).sin() * 0.5
}).collect();

// Resample the signal to the output sample rate.

let mut resampler = fixed_resample::NonRtResampler::<f32>::new(
    IN_SAMPLE_RATE,
    OUT_SAMPLE_RATE,
    1, // mono signal
    Default::default(), // default quality
);

let mut out_samples: Vec<f32> = Vec::with_capacity(resampler.out_alloc_frames(
    IN_SAMPLE_RATE,
    OUT_SAMPLE_RATE,
    in_samples.len(),
));
// (There is also a method to process non-interleaved signals.)
resampler.process_interleaved(
    &in_samples,
    // This method gets called whenever there is new resampled data.
    |data| {
        out_samples.extend_from_slice(data);
    },
    // Whether or not this is the last (or only) packet of data that
    // will be resampled. This ensures that any leftover samples in
    // the internal resampler are flushed to the output.
    true,
);

// The resulting output may have a few extra padded zero samples on the end, so
// truncate those if desired.
out_samples.resize(
    resampler.out_frames(IN_SAMPLE_RATE, OUT_SAMPLE_RATE, in_samples.len()),
    0.0,
);
```

## Realtime SPSC channel example

```rust
const IN_SAMPLE_RATE: u32 = 44100;
const OUT_SAMPLE_RATE: u32 = 48000;
const BLOCK_FRAMES: usize = 1024;
const NUM_CHANNELS: usize = 2;
const DISCARD_THRESHOLD_SECONDS: f64 = 0.35;

let (mut prod1, mut cons1) = fixed_resample::resampling_channel(
    IN_SAMPLE_RATE,
    OUT_SAMPLE_RATE,
    NUM_CHANNELS,
    Default::default(), // default configuration
);

let in_stream_interval = Duration::from_secs_f64(BLOCK_FRAMES as f64 / IN_SAMPLE_RATE as f64);
let out_stream_interval = Duration::from_secs_f64(BLOCK_FRAMES as f64 / OUT_SAMPLE_RATE as f64);

// Simulate a realtime input stream.
std::thread::spawn(move || {
    let mut phasor: f32 = 0.0;
    let phasor_inc: f32 = FREQ_HZ / IN_SAMPLE_RATE as f32;

    let mut in_buf = vec![0.0; BLOCK_FRAMES * NUM_CHANNELS];

    loop {
        // Generate a sine wave on all channels.
        for chunk in in_buf.chunks_exact_mut(NUM_CHANNELS) {
            let val = (phasor * std::f32::consts::TAU).sin() * GAIN;
            phasor = (phasor + phasor_inc).fract();

            for s in chunk.iter_mut() {
                *s = val;
            }
        }

        let frames = prod1.push_interleaved(&in_buf);

        if frames < BLOCK_FRAMES {
            println!("Overflow occured in channel 1!");
        }

        spin_sleep::sleep(in_stream_interval);
    }
});

// Simulate a realtime output stream.
std::thread::spawn(move || {
    let mut out_buf = vec![0.0; BLOCK_FRAMES * NUM_CHANNELS];

    loop {
        // If the value of [`ResamplingCons::occupied_seconds()`] is greater than the
        // given threshold in seconds, then discard the number of input frames needed to
        // bring the value back down to [`ResamplingCons::latency_seconds()`] to avoid
        // excessive overflows and reduce perceived audible glitchiness.
        let discarded_frames = cons1.discard_jitter(DISCARD_THRESHOLD_SECONDS);
        if discarded_frames > 0 {
            println!("Discarded frames in channel 1: {}", discarded_frames);
        }

        let status = cons1.read_interleaved(&mut out_buf);
        if let ReadStatus::Underflow = status {
            println!("Underflow occured in channel 1!");
        }

        spin_sleep::sleep(out_stream_interval);
    }
});
```