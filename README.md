# fixed-resample

[![Documentation](https://docs.rs/fixed-resample/badge.svg)](https://docs.rs/fixed-resample)
[![Crates.io](https://img.shields.io/crates/v/fixed-resample.svg)](https://crates.io/crates/fixed-resample)
[![License](https://img.shields.io/crates/l/fixed-resample.svg)](https://github.com/MeadowlarkDAW/fixed-resample/blob/main/LICENSE)

An easy to use crate for resampling at a fixed ratio.

It supports resampling in both realtime and in non-realtime applications, and also includes a handy realtime-safe spsc channel type that automatically resamples the input stream to match the output stream when needed.

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
    NonZeroUsize::new(1).unwrap(), // mono signal
    IN_SAMPLE_RATE,
    OUT_SAMPLE_RATE,
    Default::default(), // default quality
);

let output_frames = resampler.out_alloc_frames(in_samples.len());
let mut out_samples: Vec<f32> = Vec::with_capacity(output_frames);
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
    Some(LastPacketInfo {
        // Let the resampler know that we want an exact number of output
        // frames. Otherwise the resampler may add extra padded zeros
        // to the end.
        desired_output_frames: Some(output_frames as u64),
    }),
    // Trim the padded zeros at the beginning introduced by the internal
    // resampler.
    true, // trim_delay
);
```

## CPAL loopback example

```rust
const MAX_CHANNELS: usize = 2;

fn main() {
    let host = cpal::default_host();

    let input_device = host.default_input_device().unwrap();
    let output_device = host.default_output_device().unwrap();

    let output_config: cpal::StreamConfig = output_device.default_output_config().unwrap().into();

    // Try selecting an input config that matches the output sample rate to
    // avoid resampling.
    let mut input_config = None;
    for config in input_device.supported_input_configs().unwrap() {
        if let Some(config) = config.try_with_sample_rate(output_config.sample_rate) {
            input_config = Some(config);
            break;
        }
    }
    let input_config: cpal::StreamConfig = input_config
        .unwrap_or_else(|| input_device.default_input_config().unwrap())
        .into();

    let input_channels = input_config.channels as usize;
    let output_channels = output_config.channels as usize;

    dbg!(&input_config);
    dbg!(&output_config);

    let (mut prod, mut cons) = fixed_resample::resampling_channel::<f32, MAX_CHANNELS>(
        NonZeroUsize::new(input_channels).unwrap(),
        input_config.sample_rate.0,
        output_config.sample_rate.0,
        Default::default(),
    );

    let input_data_fn = move |data: &[f32], _: &cpal::InputCallbackInfo| {
        let status = prod.push_interleaved(data);

        match status {
            PushStatus::OverflowOccurred { num_frames_pushed: _ } => {
                eprintln!("output stream fell behind: try increasing channel capacity");
            }
            PushStatus::UnderflowCorrected { num_zero_frames_pushed: _ } => {
                eprintln!("input stream fell behind: try increasing channel latency");
            }
            _ => {}
        }
    };

    let mut tmp_buffer = vec![0.0; 8192 * input_channels];
    let output_data_fn = move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
        let frames = data.len() / output_channels;

        let status = cons.read_interleaved(&mut tmp_buffer[..frames * input_channels]);

        match status {
            ReadStatus::UnderflowOccurred { num_frames_read: _ } => {
                eprintln!("input stream fell behind: try increasing channel latency");
            }
            ReadStatus::OverflowCorrected { num_frames_discarded: _ } => {
                eprintln!("output stream fell behind: try increasing channel capacity");
            }
            _ => {}
        }

        data.fill(0.0);

        // Interleave the resampled input stream into the output stream.
        let channels = input_channels.min(output_channels);
        for ch_i in 0..channels {
            for (out_chunk, in_chunk) in data
                .chunks_exact_mut(output_channels)
                .zip(tmp_buffer.chunks_exact(input_channels))
            {
                out_chunk[ch_i] = in_chunk[ch_i];
            }
        }
    };

    let input_stream = input_device
        .build_input_stream(&input_config, input_data_fn, |_| {}, None)
        .unwrap();
    let output_stream = output_device
        .build_output_stream(&output_config, output_data_fn, |_| {}, None)
        .unwrap();

    // Play the streams.
    input_stream.play().unwrap();
    output_stream.play().unwrap();

    // Run for 10 seconds before closing.
    std::thread::sleep(std::time::Duration::from_secs(10));
}
```