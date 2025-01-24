use std::time::Duration;

use fixed_resample::ReadStatus;

const IN_SAMPLE_RATE: u32 = 48000;
const OUT_SAMPLE_RATE: u32 = 44100;
const BLOCK_FRAMES: usize = 2048;
const FREQ_HZ: f32 = 440.0;
const GAIN: f32 = 0.5;
const NUM_CHANNELS: usize = 2;

pub fn main() {
    let (mut prod, mut cons) = fixed_resample::resampling_channel(
        IN_SAMPLE_RATE,
        OUT_SAMPLE_RATE,
        NUM_CHANNELS,
        BLOCK_FRAMES,
        Default::default(), // default quality
    );

    // Simulate a realtime input/output stream.

    let in_stream_interval = Duration::from_secs_f64(BLOCK_FRAMES as f64 / IN_SAMPLE_RATE as f64);
    let out_stream_interval = Duration::from_secs_f64(BLOCK_FRAMES as f64 / OUT_SAMPLE_RATE as f64);

    let mut phasor: f32 = 0.0;
    let phasor_inc: f32 = FREQ_HZ / IN_SAMPLE_RATE as f32;
    let mut in_buf = vec![0.0; BLOCK_FRAMES * NUM_CHANNELS];
    std::thread::spawn(move || {
        loop {
            // Generate a sine wave on all channels.
            for chunk in in_buf.chunks_exact_mut(NUM_CHANNELS) {
                let val = (phasor * std::f32::consts::TAU).sin() * GAIN;
                phasor = (phasor + phasor_inc).fract();

                for s in chunk.iter_mut() {
                    *s = val;
                }
            }

            let frames = prod.push(&in_buf);

            if frames < BLOCK_FRAMES {
                eprintln!("Overflow occured!");
            }

            spin_sleep::sleep(in_stream_interval);
        }
    });

    let mut out_buf = vec![0.0; BLOCK_FRAMES * NUM_CHANNELS];
    loop {
        let status = cons.read(&mut out_buf);

        if let ReadStatus::Underflow = status {
            eprintln!("Underflow occured!");
        }

        // `out_buf` is now filled with the resampled data from the input stream.

        spin_sleep::sleep(out_stream_interval);
    }
}
