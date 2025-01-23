use std::time::Duration;

use stream_resample::ReadStatus;

const IN_SAMPLE_RATE: u32 = 48000;
const OUT_SAMPLE_RATE: u32 = 44100;
const BLOCK_FRAMES: usize = 2048;
const FREQ_HZ: f32 = 440.0;
const GAIN: f32 = 0.5;

pub fn main() {
    let (mut prod, mut cons) = stream_resample::resampling_channel(
        IN_SAMPLE_RATE,
        OUT_SAMPLE_RATE,
        1,
        BLOCK_FRAMES,
        Default::default(),
    );

    // Simulate a realtime input/output stream.

    let in_stream_interval = Duration::from_secs_f64(BLOCK_FRAMES as f64 / IN_SAMPLE_RATE as f64);
    let out_stream_interval = Duration::from_secs_f64(BLOCK_FRAMES as f64 / OUT_SAMPLE_RATE as f64);

    let mut phasor: f32 = 0.0;
    let phasor_inc: f32 = FREQ_HZ / IN_SAMPLE_RATE as f32;
    let mut in_buf = vec![0.0; BLOCK_FRAMES];
    std::thread::spawn(move || {
        loop {
            // Generate a sine wave.
            for s in in_buf.iter_mut() {
                *s = (phasor * std::f32::consts::TAU).sin() * GAIN;
                phasor = (phasor + phasor_inc).fract();
            }

            let frames = prod.push(&in_buf);

            if frames < in_buf.len() {
                eprintln!("Overflow occured!");
            }

            spin_sleep::sleep(in_stream_interval);
        }
    });

    let mut out_buf = vec![0.0; BLOCK_FRAMES];
    loop {
        let status = cons.read(&mut out_buf);

        if let ReadStatus::Underflow = status {
            eprintln!("Underflow occured!");
        }

        // `out_buf` is now filled with the resampled data from the input stream.

        spin_sleep::sleep(out_stream_interval);
    }
}
