use std::{
    num::NonZeroUsize,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

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
/// * `config` - Additional options for the resampling channel.
///
/// # Panics
///
/// Panics when any of the following are true:
///
/// * `in_sample_rate == 0`
/// * `out_sample_rate == 0`
/// * `num_channels == 0`
/// * `config.latency_seconds <= 0.0`
/// * `config.capacity_seconds <= 0.0`
pub fn resampling_channel<T: Sample>(
    in_sample_rate: u32,
    out_sample_rate: u32,
    num_channels: usize,
    config: ResamplingChannelConfig,
) -> (ResamplingProd<T>, ResamplingCons<T>) {
    let resampler = if in_sample_rate != out_sample_rate {
        Some(RtResampler::<T>::new(
            in_sample_rate,
            out_sample_rate,
            num_channels,
            true,
            config.quality,
        ))
    } else {
        None
    };

    resampling_channel_inner(
        resampler,
        in_sample_rate,
        out_sample_rate,
        num_channels,
        config,
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
/// * `config.latency_seconds <= 0.0`
/// * `config.capacity_seconds <= 0.0`
pub fn resampling_channel_custom<T: Sample>(
    resampler: impl Into<ResamplerType<T>>,
    in_sample_rate: u32,
    out_sample_rate: u32,
    num_channels: usize,
    config: ResamplingChannelConfig,
) -> (ResamplingProd<T>, ResamplingCons<T>) {
    let resampler: ResamplerType<T> = resampler.into();

    assert_eq!(resampler.num_channels(), num_channels);

    let resampler = if in_sample_rate != out_sample_rate {
        Some(RtResampler::<T>::from_custom(resampler, true))
    } else {
        None
    };

    resampling_channel_inner(
        resampler,
        in_sample_rate,
        out_sample_rate,
        num_channels,
        config,
    )
}

fn resampling_channel_inner<T: Sample>(
    resampler: Option<RtResampler<T>>,
    in_sample_rate: u32,
    out_sample_rate: u32,
    num_channels: usize,
    config: ResamplingChannelConfig,
) -> (ResamplingProd<T>, ResamplingCons<T>) {
    assert_ne!(in_sample_rate, 0);
    assert_ne!(out_sample_rate, 0);
    assert_ne!(num_channels, 0);
    assert!(config.latency_seconds > 0.0);
    assert!(config.capacity_seconds > 0.0);

    let latency_frames = ((in_sample_rate as f64 * config.latency_seconds).round() as usize).max(1);

    let buffer_capacity_frames = ((in_sample_rate as f64 * config.capacity_seconds).round()
        as usize)
        .max(latency_frames * 2);

    let (mut prod, cons) = ringbuf::HeapRb::<T>::new(buffer_capacity_frames * num_channels).split();

    // Pad the beginning of the buffer with zeros to create the desired latency.
    prod.push_slice(&vec![T::zero(); latency_frames * num_channels]);

    let reset_flag = Arc::new(AtomicBool::new(false));

    let in_sample_rate_recip = (in_sample_rate as f64).recip();

    (
        ResamplingProd {
            prod,
            num_channels: NonZeroUsize::new(num_channels).unwrap(),
            latency_seconds: config.latency_seconds,
            in_sample_rate_recip,
            reset_flag: Arc::clone(&reset_flag),
        },
        ResamplingCons {
            cons,
            resampler,
            num_channels: NonZeroUsize::new(num_channels).unwrap(),
            latency_frames,
            is_waiting_for_frames: true,
            latency_seconds: config.latency_seconds,
            in_sample_rate: in_sample_rate as f64,
            in_sample_rate_recip,
            reset_flag,
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
    latency_seconds: f64,
    in_sample_rate_recip: f64,
    reset_flag: Arc<AtomicBool>,
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

    /// An number describing the current amount of jitter in seconds between the
    /// input and output streams. A value of `0.0` means the two channels are
    /// perfectly synced, a value less than `0.0` means the input channel is
    /// slower than the input channel, and a value greater than `0.0` means the
    /// input channel is faster than the output channel.
    ///
    /// This value can be used to correct for jitter and avoid underflows/
    /// overflows. For example, if this value goes below a certain threshold,
    /// then you can push an extra packet of data to correct for the jitter.
    ///
    /// This number will be in the range `[-latency_seconds, capacity_seconds - latency_seconds]`,
    /// where `latency_seconds` and `capacity_seconds` are the values passed in
    /// [`ResamplingChannelConfig`] when this channel was constructed.
    ///
    /// Note, it is typical for the jitter value to be around plus or minus
    /// `out_max_block_frames / out_sample_rate` or  `data_frames / in_sample_rate`
    /// (whichever is higher) even when the streams are perfectly in sync
    /// (`data_frames` being the typical length in frames of a packet of data pushed
    /// to [`ResamplingProd::push`]).
    pub fn jitter_seconds(&self) -> f64 {
        ((self.prod.occupied_len() / self.num_channels.get()) as f64 * self.in_sample_rate_recip)
            - self.latency_seconds
    }

    /// Tell the consumer to clear all queued frames in the buffer.
    pub fn reset(&mut self) {
        self.reset_flag.store(true, Ordering::Relaxed);
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
    num_channels: NonZeroUsize,
    latency_frames: usize,
    is_waiting_for_frames: bool,
    latency_seconds: f64,
    in_sample_rate: f64,
    in_sample_rate_recip: f64,
    reset_flag: Arc<AtomicBool>,
}

impl<T: Sample> ResamplingCons<T> {
    /// The number of channels configured for this stream.
    pub fn num_channels(&self) -> NonZeroUsize {
        self.num_channels
    }

    /// Returns `true` if resampling is occurring, `false` if the input and output
    /// sample rates match.
    pub fn is_resampling(&self) -> bool {
        self.resampler.is_some()
    }

    /// Get the delay of the internal resampler, reported as a number of output
    /// frames.
    ///
    /// If no resampler is active, then this will return `0`.
    pub fn output_delay(&self) -> usize {
        self.resampler
            .as_ref()
            .map(|r| r.output_delay())
            .unwrap_or(0)
    }

    /// The number of frames that are currently available to read from the buffer.
    pub fn available_frames(&self) -> usize {
        self.cons.occupied_len() / self.num_channels.get()
    }

    /// An number describing the current amount of jitter in seconds between the
    /// input and output streams. A value of `0.0` means the two channels are
    /// perfectly synced, a value less than `0.0` means the input channel is
    /// slower than the input channel, and a value greater than `0.0` means the
    /// input channel is faster than the output channel.
    ///
    /// This value can be used to correct for jitter and avoid underflows/
    /// overflows. For example, if this value goes above a certain threshold,
    /// then you can read an extra packet of data or call
    /// [`ResamplingCons::discard_frames`] or [`ResamplingCons::discard_jitter`]
    /// to correct for the jitter.
    ///
    /// This number will be in the range `[-latency_seconds, capacity_seconds - latency_seconds]`,
    /// where `latency_seconds` and `capacity_seconds` are the values passed in
    /// [`ResamplingChannelConfig`] when this channel was constructed.
    ///
    /// Note, it is typical for the jitter value to be around plus or minus
    /// `out_max_block_frames / out_sample_rate` or  `data_frames / in_sample_rate`
    /// (whichever is higher) even when the streams are perfectly in sync
    /// (`data_frames` being the typical length in frames of a packet of data pushed
    /// to [`ResamplingProd::push`]).
    pub fn jitter_seconds(&self) -> f64 {
        (self.available_frames() as f64 * self.in_sample_rate_recip) - self.latency_seconds
    }

    /// Clear all queued frames in the buffer.
    pub fn reset(&mut self) {
        if let Some(resampler) = &mut self.resampler {
            resampler.reset();
        }

        self.cons.clear();

        self.is_waiting_for_frames = true;
    }

    /// Discard a certian number of input frames from the buffer. This can be used to
    /// correct for jitter and avoid overflows.
    ///
    /// This will discard `frames.min(self.available_frames())` frames.
    ///
    /// If `frames` is `None`, then the amount of frames to return the jitter count
    /// to `0.0` will be discarded.
    ///
    /// Returns the number of input frames that were discarded.
    pub fn discard_frames(&mut self, frames: usize) -> usize {
        self.cons
            .skip(frames.min(self.available_frames()) * self.num_channels.get())
            / self.num_channels.get()
    }

    /// If the value of [`ResamplingCons::jitter_seconds`] is greater than the
    /// given threshold in seconds, then discard the number of frames needed to
    /// bring the jitter value back to `0.0` to avoid overflows.
    ///
    /// Note, it is typical for the jitter value to be around plus or minus
    /// `out_max_block_frames / out_sample_rate` or  `data_frames / in_sample_rate`
    /// (whichever is higher) even when the streams are perfectly in sync
    /// (`data_frames` being the typical length in frames of a packet of data pushed
    /// to [`ResamplingProd::push`]).
    ///
    /// Returns the number of input frames that were discarded.
    pub fn discard_jitter(&mut self, threshold_seconds: f64) -> usize {
        assert!(threshold_seconds >= 0.0);

        let jitter_secs = self.jitter_seconds();

        if jitter_secs > threshold_seconds.max(0.0) {
            let frames = (jitter_secs * self.in_sample_rate).round() as usize;
            self.discard_frames(frames)
        } else {
            0
        }
    }

    /// Read from the channel and store the results into the output buffer
    /// in interleaved format.
    pub fn read(&mut self, output: &mut [T]) -> ReadStatus {
        let num_channels = self.num_channels.get();
        let out_frames = output.len() / num_channels;

        if self.reset_flag.swap(false, Ordering::Relaxed) {
            self.reset();
        }

        if self.is_waiting_for_frames {
            if self.available_frames() >= self.latency_frames {
                self.is_waiting_for_frames = false;
            } else {
                return ReadStatus::WaitingForFrames;
            }
        }

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

                        self.is_waiting_for_frames = true;
                    }
                },
                &mut output[..out_frames * num_channels],
            );
        } else {
            // Simply copy the input stream to the output.

            let samples = self
                .cons
                .pop_slice(&mut output[..out_frames * num_channels]);

            if samples < output.len() {
                status = ReadStatus::Underflow;

                output[samples..].fill(T::zero());

                self.is_waiting_for_frames = true;
            }
        }

        status
    }
}

/// The status of reading data from [`ResamplingCons::read`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadStatus {
    /// The buffer was fully filled with samples from the input
    /// stream.
    Ok,
    /// An input underflow occured. This may result in audible audio
    /// glitches.
    Underflow,
    /// The channel is waiting for a certain number of frames to be
    /// filled in the buffer before continuing after an underflow
    /// or a reset. The output will contain silence.
    WaitingForFrames,
}
