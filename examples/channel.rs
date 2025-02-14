use std::time::Duration;

use fixed_resample::{ReadStatus, ResamplingChannelConfig};

const IN_SAMPLE_RATE: u32 = 44100;
const OUT_SAMPLE_RATE: u32 = 48000;
const BLOCK_FRAMES: usize = 1024;
const FREQ_HZ: f32 = 440.0;
const GAIN: f32 = 0.5;
const NUM_CHANNELS: usize = 2;
const DISCARD_THRESHOLD_SECONDS: f64 = 0.35;
const NON_RT_POLL_INTERVAL: Duration = Duration::from_millis(15);

pub fn main() {
    // Note, you can also have the producer end be in a realtime thread and the
    // consumer end in a non-realtime thread or vice-versa.

    // --- Realtime & interleaved example ---------------------------------------------------

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

            let frames = prod1.push_interleaved(&interleaved_in_buf);

            if frames < BLOCK_FRAMES {
                println!("Overflow occured in channel 1!");
            }

            spin_sleep::sleep(in_stream_interval);
        }
    });

    // Simulate a realtime output stream.
    std::thread::spawn(move || {
        let mut interleaved_out_buf = vec![0.0; BLOCK_FRAMES * NUM_CHANNELS];

        loop {
            // If the value of [`ResamplingCons::occupied_seconds()`] is greater than the
            // given threshold in seconds, then discard the number of input frames needed to
            // bring the value back down to [`ResamplingCons::latency_seconds()`] to avoid
            // excessive overflows and reduce perceived audible glitchiness.
            //
            // For example, when compiled in debug mode without optimizations the resampler
            // is quite slow, leading to this output stream running slower than the input
            // stream, causing this to discard frames often.
            let discarded_frames = cons1.discard_jitter(DISCARD_THRESHOLD_SECONDS);
            if discarded_frames > 0 {
                println!("Discarded frames in channel 1: {}", discarded_frames);
            }

            let status = cons1.read_interleaved(&mut interleaved_out_buf);
            if let ReadStatus::Underflow = status {
                println!("Underflow occured in channel 1!");
            }

            spin_sleep::sleep(out_stream_interval);
        }
    });

    // --- Non-realtime & de-interleaved example --------------------------------------------

    let (mut prod2, mut cons2) = fixed_resample::resampling_channel(
        IN_SAMPLE_RATE,
        OUT_SAMPLE_RATE,
        NUM_CHANNELS,
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
    loop {
        // The amount of frames in 1/4 of a second.
        let packet_frames = OUT_SAMPLE_RATE as usize / 4;
        let mut deinterleaved_out_buf: Vec<Vec<f32>> = (0..NUM_CHANNELS)
            .map(|_| vec![0.0; packet_frames])
            .collect();

        while cons2.available_frames() >= packet_frames {
            let status = cons2.read(&mut deinterleaved_out_buf, 0..packet_frames);
            if let ReadStatus::Underflow = status {
                println!("Underflow occured in channel 2!");
            }
        }

        std::thread::sleep(NON_RT_POLL_INTERVAL);
    }
}
