use std::num::NonZeroUsize;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use fixed_resample::ReadStatus;

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
        let pushed_frames = prod.push_interleaved(data);

        if pushed_frames * input_channels < data.len() {
            eprintln!("output stream fell behind: try increasing channel capacity");
        }
    };

    let mut tmp_buffer = vec![0.0; 8192 * input_channels];
    let output_data_fn = move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
        let frames = data.len() / output_channels;

        let status = cons.read_interleaved(&mut tmp_buffer[..frames * input_channels]);

        if let ReadStatus::Underflow { .. } = status {
            eprintln!("input stream fell behind: try increasing channel latency");
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
        .build_input_stream(&input_config, input_data_fn, err_fn, None)
        .unwrap();
    let output_stream = output_device
        .build_output_stream(&output_config, output_data_fn, err_fn, None)
        .unwrap();

    // Play the streams.
    input_stream.play().unwrap();
    output_stream.play().unwrap();

    // Run for 10 seconds before closing.
    std::thread::sleep(std::time::Duration::from_secs(10));
}

fn err_fn(err: cpal::StreamError) {
    eprintln!("an error occurred on stream: {}", err);
}
