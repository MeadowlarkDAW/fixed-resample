use std::{num::NonZero, time::Duration};

use fixed_resample::ReadStatus;

const IN_SAMPLE_RATE: u32 = 44100;
const OUT_SAMPLE_RATE: u32 = 48000;
const BLOCK_FRAMES: usize = 1024;
const FREQ_HZ: f32 = 440.0;
const GAIN: f32 = 0.5;
const NUM_CHANNELS: usize = 2;
const DISCARD_THRESHOLD_SECONDS: f64 = 0.15;

pub fn main() {
    let (mut prod1, mut cons1) = fixed_resample::resampling_channel(
        IN_SAMPLE_RATE,
        OUT_SAMPLE_RATE,
        NUM_CHANNELS,
        Default::default(), // default configuration
    );

    // Demonstrate deinterleaved usage.
    let (mut prod2, mut cons2) = fixed_resample::resampling_channel(
        IN_SAMPLE_RATE,
        OUT_SAMPLE_RATE,
        NUM_CHANNELS,
        Default::default(), // default configuration
    );

    // Simulate a realtime input/output stream with independent clocks.

    let in_stream_interval = Duration::from_secs_f64(BLOCK_FRAMES as f64 / IN_SAMPLE_RATE as f64);
    let out_stream_interval = Duration::from_secs_f64(BLOCK_FRAMES as f64 / OUT_SAMPLE_RATE as f64);

    let mut phasor: f32 = 0.0;
    let phasor_inc: f32 = FREQ_HZ / IN_SAMPLE_RATE as f32;
    let mut interleaved_in_buf = vec![0.0; BLOCK_FRAMES * NUM_CHANNELS];
    let mut deinterleaved_in_buf: Vec<Vec<f32>> =
        (0..NUM_CHANNELS).map(|_| vec![0.0; BLOCK_FRAMES]).collect();
    std::thread::spawn(move || {
        let mut print_jitter_count: u64 = 0;

        loop {
            // Print the calculated jitter every 32 loop cycles.
            //
            // This jitter value can be used to avoid underflows/overflows by
            // pushing more/less packets of data when this value reaches a
            // certain threshold.
            if print_jitter_count % 32 == 0 {
                dbg!(prod1.jitter_seconds());
            }
            print_jitter_count += 1;

            // Generate a sine wave on all channels.
            for chunk in interleaved_in_buf.chunks_exact_mut(NUM_CHANNELS) {
                let val = (phasor * std::f32::consts::TAU).sin() * GAIN;
                phasor = (phasor + phasor_inc).fract();

                for s in chunk.iter_mut() {
                    *s = val;
                }
            }

            // Interleaved usage
            let frames = prod1.push_interleaved(&interleaved_in_buf);
            if frames < BLOCK_FRAMES {
                eprintln!("Overflow occured in channel 1!");
            }

            fixed_resample::interleave::deinterleave(
                &interleaved_in_buf,
                &mut deinterleaved_in_buf,
                NonZero::new(NUM_CHANNELS).unwrap(),
                0..BLOCK_FRAMES,
            );

            // Deinterleaved usage
            let frames = prod2.push(&deinterleaved_in_buf, 0..BLOCK_FRAMES);
            if frames < BLOCK_FRAMES {
                eprintln!("Overflow occured in channel 2!");
            }

            spin_sleep::sleep(in_stream_interval);
        }
    });

    let mut interleaved_out_buf = vec![0.0; BLOCK_FRAMES * NUM_CHANNELS];
    let mut deinterleaved_out_buf: Vec<Vec<f32>> =
        (0..NUM_CHANNELS).map(|_| vec![0.0; BLOCK_FRAMES]).collect();
    loop {
        // This jitter value can be also be gotten from the consumer side.
        //
        // This can be used to avoid overflows by reading more packets or
        // by discarding samples from the buffer when this value reaches a
        // certain threshold.
        let _ = cons1.jitter_seconds();
        let _ = cons2.jitter_seconds();

        // This method can also be used to automatically discard samples if
        // the jitter value goes above the given threshold.
        //
        // For example, in debug mode the resampler is quite slow, leading
        // to this output stream running slower than the input stream and
        // causing this to discard frames often.
        let discarded_frames = cons1.discard_jitter(DISCARD_THRESHOLD_SECONDS);
        if discarded_frames > 0 {
            println!("Discarded frames in channel 1: {}", discarded_frames);
        }
        let discarded_frames = cons2.discard_jitter(DISCARD_THRESHOLD_SECONDS);
        if discarded_frames > 0 {
            println!("Discarded frames in channel 2: {}", discarded_frames);
        }

        // Interleaved usage
        let status = cons1.read_interleaved(&mut interleaved_out_buf);
        if let ReadStatus::Underflow = status {
            eprintln!("Underflow occured in channel 1!");
        }

        // Deinterleaved usage
        let status = cons2.read(&mut deinterleaved_out_buf, 0..BLOCK_FRAMES);
        if let ReadStatus::Underflow = status {
            eprintln!("Underflow occured in channel 2!");
        }

        // `out_buf` is now filled with the resampled data from the input stream.

        spin_sleep::sleep(out_stream_interval);
    }
}
