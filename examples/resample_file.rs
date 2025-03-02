use std::{i16, num::NonZeroUsize};

use clap::Parser;
use fixed_resample::{FixedResampler, LastPacketInfo, ResampleQuality};
use hound::SampleFormat;

const MAX_CHANNELS: usize = 2;

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
        "high" => ResampleQuality::High,
        s => {
            eprintln!("unkown quality type: {}", s);
            println!("Valid options are [\"low\", and \"high\"]");
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

    if spec.channels as usize > MAX_CHANNELS {
        eprintln!(
            "Only wav files with up to {} channels are supported in this example.",
            MAX_CHANNELS
        );
    }

    let num_channels = NonZeroUsize::new(spec.channels as usize).unwrap();

    let in_samples: Vec<f32> = reader
        .samples::<i16>()
        .map(|s| s.unwrap() as f32 / (i16::MAX as f32))
        .collect();

    // --- Resample the contents into the output ---------------------------------------------

    let mut resampler = FixedResampler::<f32, MAX_CHANNELS>::new(
        num_channels,
        spec.sample_rate,
        args.target_sample_rate,
        quality,
        true, // interleaved
    );

    // Allocate an output buffer with the needed number of frames.
    let input_frames = in_samples.len() / num_channels.get();
    let output_frames = resampler.out_alloc_frames(input_frames as u64) as usize;
    let mut out_samples = Vec::new();
    // Since we know we don't need any more samples than this, it's typically a
    // good idea to try and save memory by using `reserve_exact`.
    out_samples.reserve_exact(output_frames * num_channels.get());

    resampler.process_interleaved(
        &in_samples,
        // This method gets called whenever there is new resampled data.
        |data| {
            out_samples.extend_from_slice(data);
        },
        // Whether or not this is the last (or only) packet of data that
        // will be resampled. This ensures that any leftover samples in
        // the internal resampler are flushed to the output.
        Some(LastPacketInfo {
            // Let the resampler know that we want an exact number of output
            // frames. Otherwise the resampler may add extra padded zeros
            // to the end.
            desired_output_frames: Some(output_frames as u64),
        }),
        // Trim the padded zeros at the beginning introduced by the internal
        // resampler.
        true, // trim_delay
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
