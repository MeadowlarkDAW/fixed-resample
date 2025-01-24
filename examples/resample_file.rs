use std::i16;

use clap::Parser;
use fixed_resample::{NonRtResampler, ResampleQuality};
use hound::SampleFormat;

#[derive(Parser)]
struct Args {
    /// The path to the audio file to play
    path: std::path::PathBuf,

    /// The target sample rate.
    target_sample_rate: u32,

    /// The resample quality. Valid options are ["low", and "normal"].
    quality: String,
}

pub fn main() {
    let args = Args::parse();

    let quality = match args.quality.as_str() {
        "low" => ResampleQuality::Low,
        "normal" => ResampleQuality::Normal,
        s => {
            eprintln!("unkown quality type: {}", s);
            println!("Valid options are [\"low\", and \"normal\"]");
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

    // --- Resample the contents into the output ---------------------------------------------

    let mut resampler = NonRtResampler::<f32>::new(
        spec.sample_rate,
        args.target_sample_rate,
        spec.channels as usize,
        quality,
    );

    let mut out_samples: Vec<f32> = Vec::with_capacity(resampler.out_alloc_frames(
        spec.sample_rate,
        args.target_sample_rate,
        in_samples.len(),
    ));
    resampler.process_interleaved(
        &in_samples,
        // This method gets called whenever there is new resampled data.
        |data| {
            out_samples.extend_from_slice(data);
        },
        // Whether or not this is the last (or only) packet of data that
        // will be resampled. This ensures that any leftover samples in
        // the internal resampler are flushed to the output.
        true,
    );

    // The resulting output may have a few extra padded zero samples on the end, so
    // truncate those if desired.
    out_samples.resize(
        resampler.out_frames(spec.sample_rate, args.target_sample_rate, in_samples.len()),
        0.0,
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

    for &s in out_samples.iter() {
        writer.write_sample((s * i16::MAX as f32) as i16).unwrap();
    }

    writer.finalize().unwrap();
}
