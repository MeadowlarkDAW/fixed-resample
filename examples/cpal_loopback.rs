use std::num::NonZeroUsize;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use fixed_resample::{PushStatus, ReadStatus};

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
            // All samples were successfully pushed to the channel.
            PushStatus::Ok => {}
            // The output stream is not yet ready to read samples from the channel. No
            // samples have been pushed to the channel.
            PushStatus::OutputNotReady => {}
            // An overflow occured due to the input stream running faster than the output
            // stream.
            PushStatus::OverflowOccurred {
                num_frames_pushed: _,
            } => {
                eprintln!("output stream fell behind: try increasing channel capacity");
            }
            // An underflow occured due to the output stream running faster than the
            // input stream.
            //
            // All of the samples were successfully pushed to the channel, however extra
            // zero samples were also pushed to the channel to correct for the jitter.
            PushStatus::UnderflowCorrected {
                num_zero_frames_pushed: _,
            } => {
                eprintln!("input stream fell behind: try increasing channel latency");
            }
        }
    };

    let mut tmp_buffer = vec![0.0; 8192 * input_channels];
    let output_data_fn = move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
        let frames = data.len() / output_channels;

        let status = cons.read_interleaved(&mut tmp_buffer[..frames * input_channels]);

        match status {
            // The output buffer was fully filled with samples from the channel.
            ReadStatus::Ok => {}
            // The input stream is not yet ready to push samples to the channel.
            // The output buffer was filled with zeros.
            ReadStatus::InputNotReady => {}
            // An underflow occured due to the output stream running faster than the input
            // stream.
            ReadStatus::UnderflowOccurred { num_frames_read: _ } => {
                eprintln!("input stream fell behind: try increasing channel latency");
            }
            // An overflow occured due to the input stream running faster than the output
            // stream
            //
            // All of the samples in the output buffer were successfully filled with samples,
            // however a number of frames have also been discarded to correct for the jitter.
            ReadStatus::OverflowCorrected {
                num_frames_discarded: _,
            } => {
                eprintln!("output stream fell behind: try increasing channel capacity");
            }
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
