use std::{num::NonZeroUsize, time::Duration};

use fixed_resample::{ReadStatus, ResamplingChannelConfig};

const IN_SAMPLE_RATE: u32 = 44100;
const OUT_SAMPLE_RATE: u32 = 48000;
const BLOCK_FRAMES: usize = 1024;
const FREQ_HZ: f32 = 440.0;
const GAIN: f32 = 0.5;
const NUM_CHANNELS: usize = 2;
const MAX_CHANNELS: usize = 2;
const DISCARD_THRESHOLD_SECONDS: f64 = 0.35;
const NON_RT_POLL_INTERVAL: Duration = Duration::from_millis(15);

pub fn main() {
    // Note, you can also have the producer end be in a realtime thread and the
    // consumer end in a non-realtime thread (and vice-versa).

    // --- Realtime & interleaved example ---------------------------------------------------

    let (mut prod1, mut cons1) = fixed_resample::resampling_channel::<f32, MAX_CHANNELS>(
        NonZeroUsize::new(NUM_CHANNELS).unwrap(),
        IN_SAMPLE_RATE,
        OUT_SAMPLE_RATE,
        Default::default(), // default configuration
    );

    let in_stream_interval = Duration::from_secs_f64(BLOCK_FRAMES as f64 / IN_SAMPLE_RATE as f64);
    let out_stream_interval = Duration::from_secs_f64(BLOCK_FRAMES as f64 / OUT_SAMPLE_RATE as f64);

    // Simulate a realtime input stream (i.e. a CPAL input stream).
    std::thread::spawn(move || {
        let mut phasor: f32 = 0.0;
        let phasor_inc: f32 = FREQ_HZ / IN_SAMPLE_RATE as f32;

        let mut interleaved_in_buf = vec![0.0; BLOCK_FRAMES * NUM_CHANNELS];

        loop {
            // Generate a sine wave on all channels.
            for chunk in interleaved_in_buf.chunks_exact_mut(NUM_CHANNELS) {
                let val = (phasor * std::f32::consts::TAU).sin() * GAIN;
                phasor = (phasor + phasor_inc).fract();

                for s in chunk.iter_mut() {
                    *s = val;
                }
            }

            // If the channel is empty due to an underflow, fill the channel with
            // [`ResamplingProd::latency_seconds`] output frames to avoid excessive
            // underflows in the future and reduce the percieved audible glitchiness.
            //
            // For example, when compiled in debug mode without optimizations, the resampler
            // is quite slow, leading to frequent underflows in this example.
            prod1.correct_underflows();

            let frames = prod1.push_interleaved(&interleaved_in_buf);

            // If this value is less then the size of our packet, then it means an overflow
            // has occurred.
            if frames < BLOCK_FRAMES {
                println!("Overflow occured in channel 1!");
            }

            spin_sleep::sleep(in_stream_interval);
        }
    });

    // Simulate a realtime output stream (i.e. a CPAL output stream).
    std::thread::spawn(move || {
        let mut interleaved_out_buf = vec![0.0; BLOCK_FRAMES * NUM_CHANNELS];

        loop {
            // If the value of [`ResamplingCons::occupied_seconds()`] is greater than the
            // given threshold in seconds, then discard the number of output frames needed to
            // bring the value back down to [`ResamplingCons::latency_seconds()`] to avoid
            // excessive overflows and reduce perceived audible glitchiness.
            let discarded_frames = cons1.discard_jitter(DISCARD_THRESHOLD_SECONDS);
            if discarded_frames > 0 {
                println!("Discarded frames in channel 1: {}", discarded_frames);
            }

            let status = cons1.read_interleaved(&mut interleaved_out_buf);

            // Note, when compiled in debug mode without optimizations, the resampler
            // is quite slow, leading to frequent underflows in this example.
            if let ReadStatus::Underflow { num_frames_dropped } = status {
                println!(
                    "Underflow occured in channel 1! Number of frames dropped: {}",
                    num_frames_dropped
                );
            }

            spin_sleep::sleep(out_stream_interval);
        }
    });

    // --- Non-realtime & de-interleaved example --------------------------------------------

    let (mut prod2, mut cons2) = fixed_resample::resampling_channel::<f32, MAX_CHANNELS>(
        NonZeroUsize::new(NUM_CHANNELS).unwrap(),
        IN_SAMPLE_RATE,
        OUT_SAMPLE_RATE,
        ResamplingChannelConfig {
            // In this channel we are pushing packets of data that are 1 second long,
            // so we need to increase the capacity of this channel to be at least
            // twice that.
            capacity_seconds: 2.0,
            // In this channel we are reading packets of data that are a quarter second
            // long, so we need to increase the latency of the channel to be at least
            // that (here we go for twice that to be safe).
            latency_seconds: 0.5,
            ..Default::default()
        },
    );

    // Simulate a non-realtime input stream (i.e. streaming data from a network).
    std::thread::spawn(move || {
        let mut phasor: f32 = 0.0;
        let phasor_inc: f32 = FREQ_HZ / IN_SAMPLE_RATE as f32;

        // The amount of frames in 1 second.
        let packet_frames = IN_SAMPLE_RATE as usize;
        let mut deinterleaved_in_buf: Vec<Vec<f32>> = (0..NUM_CHANNELS)
            .map(|_| vec![0.0; packet_frames])
            .collect();

        loop {
            // Detect when a new packet of data should be pushed.
            //
            // Alternatively you could do:
            // while prod2.available_frames() >= packet_frames {
            while prod2.occupied_seconds() < prod2.latency_seconds() {
                // Generate a sine wave on all channels.
                for i in 0..packet_frames {
                    let val = (phasor * std::f32::consts::TAU).sin() * GAIN;
                    phasor = (phasor + phasor_inc).fract();

                    for ch in deinterleaved_in_buf.iter_mut() {
                        ch[i] = val;
                    }
                }

                // Push a new packet of data to the stream.
                prod2.push(&deinterleaved_in_buf, 0..packet_frames);
            }

            std::thread::sleep(NON_RT_POLL_INTERVAL);
        }
    });

    // Simulate a non-realtime output stream (i.e. streaming data to a network).
    std::thread::spawn(move || {
        loop {
            // The amount of frames in 1/4 of a second.
            let packet_frames = OUT_SAMPLE_RATE as usize / 4;
            let mut deinterleaved_out_buf: Vec<Vec<f32>> = (0..NUM_CHANNELS)
                .map(|_| vec![0.0; packet_frames])
                .collect();

            while cons2.available_frames() >= packet_frames {
                let status = cons2.read(&mut deinterleaved_out_buf, 0..packet_frames);
                if let ReadStatus::Underflow { num_frames_dropped } = status {
                    println!(
                        "Underflow occured in channel 2! Number of frames dropped {}",
                        num_frames_dropped
                    );
                }
            }

            std::thread::sleep(NON_RT_POLL_INTERVAL);
        }
    });

    // Run for 10 seconds before closing.
    std::thread::sleep(std::time::Duration::from_secs(10));
}
