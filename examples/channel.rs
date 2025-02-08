use std::time::Duration;

use fixed_resample::ReadStatus;

const IN_SAMPLE_RATE: u32 = 48000;
const OUT_SAMPLE_RATE: u32 = 44100;
const BLOCK_FRAMES: usize = 2048;
const FREQ_HZ: f32 = 440.0;
const GAIN: f32 = 0.5;
const NUM_CHANNELS: usize = 2;
const DISCARD_THRESHOLD_SECONDS: f64 = 0.15;

pub fn main() {
    let (mut prod, mut cons) = fixed_resample::resampling_channel(
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
    let mut in_buf = vec![0.0; BLOCK_FRAMES * NUM_CHANNELS];
    std::thread::spawn(move || {
        let mut print_jitter_count: u64 = 0;

        loop {
            // Print the calculated jitter every 32 loop cycles.
            //
            // This jitter value can be used to avoid underflows/overflows by
            // pushing more/less packets of data when this value reaches a
            // certain threshold.
            if print_jitter_count % 32 == 0 {
                dbg!(prod.jitter_seconds());
            }
            print_jitter_count += 1;

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
        // This jitter value can be also be gotten from the consumer side.
        //
        // This can be used to avoid overflows by reading more packets or
        // by discarding samples from the buffer when this value reaches a
        // certain threshold.
        let _ = cons.jitter_seconds();

        // This method can also be used to automatically discard samples if
        // the jitter value goes above the given threshold.
        //
        // For example, in debug mode the resampler is quite slow, leading
        // to this output stream running slower than the input stream and
        // causing this to discard frames often.
        let discarded_frames = cons.discard_jitter(DISCARD_THRESHOLD_SECONDS);
        if discarded_frames > 0 {
            println!("Discarded frames: {}", discarded_frames);
        }

        let status = cons.read(&mut out_buf);

        if let ReadStatus::Underflow = status {
            eprintln!("Underflow occured!");
        }

        // `out_buf` is now filled with the resampled data from the input stream.

        spin_sleep::sleep(out_stream_interval);
    }
}
