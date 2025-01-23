use std::i16;

use clap::Parser;
use hound::SampleFormat;
use stream_resample::{NonRtResampler, ResampleQuality};

#[derive(Parser)]
struct Args {
    /// The path to the audio file to play
    path: std::path::PathBuf,

    /// The target sample rate.
    target_sample_rate: u32,

    /// The resample quality. Valid options are ["low", "normal", "high", and "very_high"].
    quality: String,
}

pub fn main() {
    let args = Args::parse();

    let quality = match args.quality.as_str() {
        "low" => ResampleQuality::Low,
        "normal" => ResampleQuality::Normal,
        "high" => ResampleQuality::High,
        "very_high" => ResampleQuality::VeryHigh,
        s => {
            eprintln!("unkown quality type: {}", s);
            println!("Valid options are [\"low\", \"normal\", \"high\", and \"very_high\"]");
            return;
        }
    };

    // --- Load the audio file with hound ----------------------------------------------------

    let mut reader = match hound::WavReader::open(&args.path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to open file: {}", e);
            return;
        }
    };

    let spec = reader.spec();

    if spec.channels > 2 {
        eprintln!("Wav files with more than 2 channels are not supported in this example.");
        return;
    }
    if spec.sample_format != SampleFormat::Int || spec.sample_format != SampleFormat::Int {
        eprintln!("Only wav files with 16 bit sample formats are supported in this example.");
        return;
    }

    if spec.sample_rate == args.target_sample_rate {
        println!("Already the same sample rate.");
        return;
    }

    let in_samples: Vec<f32> = reader
        .samples::<i16>()
        .map(|s| s.unwrap() as f32 / (i16::MAX as f32))
        .collect();

    let in_samples = if reader.spec().channels == 1 {
        vec![in_samples]
    } else {
        // Deinterleave the samples.
        let mut v1 = Vec::with_capacity(in_samples.len() / 2);
        let mut v2 = Vec::with_capacity(in_samples.len() / 2);
        for s in in_samples.chunks_exact(2) {
            v1.push(s[0]);
            v2.push(s[1]);
        }
        vec![v1, v2]
    };

    // --- Create the resampler --------------------------------------------------------------

    let mut resampler = NonRtResampler::<f32>::new(
        spec.sample_rate,
        args.target_sample_rate,
        spec.channels as usize,
        quality,
    );

    // --- Resample the contents into the output ---------------------------------------------

    let mut out_samples: Vec<Vec<f32>> = (0..spec.channels).map(|_| Vec::new()).collect();
    resampler.process(
        &in_samples,
        // This method gets called whenever there is new resampled data.
        |data, frames| {
            for (out_ch, data_ch) in out_samples.iter_mut().zip(data.iter()) {
                // Note, `frames` may be less than the length of `data`.
                out_ch.extend_from_slice(&data_ch[..frames]);
            }
        },
        // Whether or not this is the last (or only) packet of data that
        // will be resampled. This ensures that any leftover samples in
        // the internal resampler are flushed to the output.
        true,
    );

    // --- Write the resampled data to a new wav file ----------------------------------------

    let mut new_file = args.path.clone();
    let file_name = args.path.file_stem().unwrap().to_str().unwrap();
    new_file.set_file_name(format!("{}_res_{}.wav", file_name, args.target_sample_rate));

    let new_spec = hound::WavSpec {
        channels: spec.channels,
        sample_rate: args.target_sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer = match hound::WavWriter::create(new_file, new_spec) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("Failed to create file: {}", e);
            return;
        }
    };

    if spec.channels == 1 {
        for &s in out_samples[0].iter() {
            writer.write_sample(s).unwrap();
        }
    } else {
        // Interleave the samples.
        for (&s1, &s2) in out_samples[0].iter().zip(out_samples[1].iter()) {
            writer.write_sample((s1 * i16::MAX as f32) as i16).unwrap();
            writer.write_sample((s2 * i16::MAX as f32) as i16).unwrap();
        }
    }

    writer.finalize().unwrap();
}
