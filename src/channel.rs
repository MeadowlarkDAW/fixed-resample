use std::num::NonZeroUsize;

use ringbuf::traits::{Consumer, Observer, Producer, Split};
use rubato::Sample;

use crate::{ResampleQuality, ResamplerType, RtResampler};

/// Additional options for a resampling channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResamplingChannelConfig {
    /// The amount of latency added in seconds between the input stream and the
    /// output stream. If this value is too small, then underflows may occur.
    ///
    /// The default value is `0.15` (150 ms).
    pub latency_seconds: f64,

    /// The capacity of the channel in seconds. If this is too small, then
    /// overflows may occur. This should be at least twice as large as
    /// `latency_seconds`.
    ///
    /// The default value is `0.4` (400 ms).
    pub capacity_seconds: f64,

    /// The quality of the resampling alrgorithm to use if needed.
    ///
    /// The default value is `ResampleQuality::Normal`.
    pub quality: ResampleQuality,
}

impl Default for ResamplingChannelConfig {
    fn default() -> Self {
        Self {
            latency_seconds: 0.15,
            capacity_seconds: 0.4,
            quality: ResampleQuality::Normal,
        }
    }
}

/// Create a new realtime-safe spsc channel for sending samples across streams.
///
/// If the input and output samples rates differ, then this will automatically
/// resample the input stream to match the output stream. If the sample rates
/// match, then no resampling will occur.
///
/// Internally this uses the `ringbuf` crate.
///
/// * `in_sample_rate` - The sample rate of the input stream.
/// * `out_sample_rate` - The sample rate of the output stream.
/// * `num_channels` - The number of channels in the stream.
/// * `out_max_block_frames` - The maximum number of frames that can be read
/// in a single call to [`ResamplingCons::read`].
/// * `config` - Additional options for the resampling channel.
///
/// # Panics
///
/// Panics when any of the following are true:
///
/// * `in_sample_rate == 0`
/// * `out_sample_rate == 0`
/// * `num_channels == 0`
/// * `out_max_block_frames == 0`,
/// * `config.latency_seconds <= 0.0`
/// * `config.capacity_seconds <= 0.0`
pub fn resampling_channel<T: Sample>(
    in_sample_rate: u32,
    out_sample_rate: u32,
    num_channels: usize,
    out_max_block_frames: usize,
    config: ResamplingChannelConfig,
) -> (ResamplingProd<T>, ResamplingCons<T>) {
    assert_ne!(in_sample_rate, 0);
    assert_ne!(out_sample_rate, 0);
    assert_ne!(out_max_block_frames, 0);
    assert_ne!(num_channels, 0);
    assert!(config.latency_seconds > 0.0);
    assert!(config.capacity_seconds > 0.0);

    let latency_frames = ((in_sample_rate as f64 * config.latency_seconds).round() as usize).max(1);

    let buffer_capacity_frames = ((in_sample_rate as f64 * config.capacity_seconds).round()
        as usize)
        .max(latency_frames * 2);

    let (mut prod, cons) = ringbuf::HeapRb::<T>::new(buffer_capacity_frames * num_channels).split();

    // Add latency by initializing the ring buffer with zeros. This is needed
    // to avoid underflows.
    prod.push_slice(&vec![T::zero(); latency_frames * num_channels]);

    let resampler = if in_sample_rate != out_sample_rate {
        Some(RtResampler::<T>::new(
            in_sample_rate,
            out_sample_rate,
            num_channels,
            out_max_block_frames,
            true,
            config.quality,
        ))
    } else {
        None
    };

    (
        ResamplingProd {
            prod,
            num_channels: NonZeroUsize::new(num_channels).unwrap(),
        },
        ResamplingCons {
            cons,
            resampler,
            max_block_frames: out_max_block_frames,
            num_channels: NonZeroUsize::new(num_channels).unwrap(),
        },
    )
}

/// Create a new realtime-safe spsc channel for sending samples across streams
/// using the custom resampler.
///
/// If the input and output samples rates differ, then this will automatically
/// resample the input stream to match the output stream. If the sample rates
/// match, then no resampling will occur.
///
/// Internally this uses the `ringbuf` crate.
///
/// * `resampler` - The custom rubato resampler.
/// * `in_sample_rate` - The sample rate of the input stream.
/// * `out_sample_rate` - The sample rate of the output stream.
/// * `num_channels` - The number of channels in the stream.
/// * `out_max_block_frames` - The maximum number of frames that can be read
/// in a single call to [`ResamplingCons::read`].
/// * `config` - Additional options for the resampling channel. Note that
/// `config.quality` will be ignored.
///
/// # Panics
///
/// Panics when any of the following are true:
///
/// * `resampler.num_channels() != num_channels`
/// * `in_sample_rate == 0`
/// * `out_sample_rate == 0`
/// * `num_channels == 0`
/// * `out_max_block_frames == 0`,
/// * `config.latency_seconds <= 0.0`
/// * `config.capacity_seconds <= 0.0`
pub fn resampling_channel_custom<T: Sample>(
    resampler: impl Into<ResamplerType<T>>,
    in_sample_rate: u32,
    out_sample_rate: u32,
    num_channels: usize,
    out_max_block_frames: usize,
    config: ResamplingChannelConfig,
) -> (ResamplingProd<T>, ResamplingCons<T>) {
    let resampler: ResamplerType<T> = resampler.into();

    assert_eq!(resampler.num_channels(), num_channels);
    assert_ne!(in_sample_rate, 0);
    assert_ne!(out_sample_rate, 0);
    assert_ne!(out_max_block_frames, 0);
    assert_ne!(num_channels, 0);
    assert!(config.latency_seconds > 0.0);
    assert!(config.capacity_seconds > 0.0);

    let latency_frames = ((in_sample_rate as f64 * config.latency_seconds).round() as usize).max(1);

    let buffer_capacity_frames = ((in_sample_rate as f64 * config.capacity_seconds).round()
        as usize)
        .max(latency_frames * 2);

    let (mut prod, cons) = ringbuf::HeapRb::<T>::new(buffer_capacity_frames * num_channels).split();

    // Add latency by initializing the ring buffer with zeros. This is needed
    // to avoid underflows.
    prod.push_slice(&vec![T::zero(); latency_frames * num_channels]);

    let resampler = if in_sample_rate != out_sample_rate {
        Some(RtResampler::<T>::from_custom(
            resampler,
            out_max_block_frames,
            true,
        ))
    } else {
        None
    };

    (
        ResamplingProd {
            prod,
            num_channels: NonZeroUsize::new(num_channels).unwrap(),
        },
        ResamplingCons {
            cons,
            resampler,
            max_block_frames: out_max_block_frames,
            num_channels: NonZeroUsize::new(num_channels).unwrap(),
        },
    )
}

/// The producer end of a realtime-safe spsc channel for sending samples across
/// streams.
///
/// If the input and output samples rates differ, then this will automatically
/// resample the input stream to match the output stream. If the sample rates
/// match, then no resampling will occur.
///
/// Internally this uses the `ringbuf` crate.
pub struct ResamplingProd<T: Sample> {
    prod: ringbuf::HeapProd<T>,
    num_channels: NonZeroUsize,
}

impl<T: Sample> ResamplingProd<T> {
    /// Push the given data in interleaved format.
    ///
    /// Returns the number of frames (not samples) that were successfully pushed.
    /// If this number is less than the number of frames in `data`, then it means
    /// an overflow has occured.
    pub fn push(&mut self, data: &[T]) -> usize {
        let data_frames = data.len() / self.num_channels.get();

        let pushed_samples = self
            .prod
            .push_slice(&data[..data_frames * self.num_channels.get()]);

        pushed_samples / self.num_channels.get()
    }

    /// Returns the number of frames that are currently available to be pushed
    /// to the buffer.
    pub fn available_frames(&self) -> usize {
        self.prod.vacant_len() / self.num_channels.get()
    }

    /// The number of channels configured for this stream.
    pub fn num_channels(&self) -> NonZeroUsize {
        self.num_channels
    }
}

/// The consumer end of a realtime-safe spsc channel for sending samples across
/// streams.
///
/// If the input and output samples rates differ, then this will automatically
/// resample the input stream to match the output stream. If the sample rates
/// match, then no resampling will occur.
///
/// Internally this uses the `ringbuf` crate.
pub struct ResamplingCons<T: Sample> {
    cons: ringbuf::HeapCons<T>,
    resampler: Option<RtResampler<T>>,
    max_block_frames: usize,
    num_channels: NonZeroUsize,
}

impl<T: Sample> ResamplingCons<T> {
    /// The number of channels configured for this stream.
    pub fn num_channels(&self) -> NonZeroUsize {
        self.num_channels
    }

    /// The maximum number of frames that can be read in a single call to
    /// [`ResamplingCons::read`].
    pub fn max_block_frames(&self) -> usize {
        self.max_block_frames
    }

    /// Read from the channel and store the results into the output buffer
    /// in interleaved format.
    ///
    /// # Panics
    ///
    /// Panics if the number of frames in `output` is greater than
    /// [`ResamplingCons::max_block_frames`].
    pub fn read(&mut self, output: &mut [T]) -> ReadStatus {
        let out_frames = output.len() / self.num_channels.get();

        assert!(out_frames <= self.max_block_frames);

        let mut status = ReadStatus::Ok;

        if let Some(resampler) = &mut self.resampler {
            resampler.process_interleaved(
                |in_buf| {
                    // Completely fill the buffer with new data.
                    // If the requested number of samples cannot be appended (i.e.
                    // an underflow occured), then fill the rest with zeros.

                    let samples = self.cons.pop_slice(in_buf);

                    if samples < in_buf.len() {
                        status = ReadStatus::Underflow;

                        in_buf[samples..].fill(T::zero());
                    }
                },
                output,
            );
        } else {
            // Simply copy the input stream to the output.

            let samples = self
                .cons
                .pop_slice(&mut output[..out_frames * self.num_channels.get()]);

            if samples < output.len() {
                status = ReadStatus::Underflow;

                output[samples..].fill(T::zero());
            }
        }

        status
    }
}

/// The status of reading data from [`ResamplingCons::read`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadStatus {
    /// No problems.
    Ok,
    /// An input underflow occured. This may result in audible audio
    /// glitches.
    Underflow,
}
