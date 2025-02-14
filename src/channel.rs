use std::{
    num::{NonZeroU32, NonZeroUsize},
    ops::Range,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use ringbuf::traits::{Consumer, Observer, Producer, Split};

#[cfg(feature = "resampler")]
use rubato::Sample;

#[cfg(feature = "resampler")]
use crate::{ResampleQuality, ResamplerType, RtResampler};

/// The trait governing a single sample.
///
/// There are two types which implements this trait so far:
/// * [f32]
/// * [f64]
#[cfg(not(feature = "resampler"))]
pub trait Sample
where
    Self: Copy + Send,
{
    fn zero() -> Self;
}

#[cfg(not(feature = "resampler"))]
impl Sample for f32 {
    fn zero() -> Self {
        0.0
    }
}

#[cfg(not(feature = "resampler"))]
impl Sample for f64 {
    fn zero() -> Self {
        0.0
    }
}

/// Additional options for a resampling channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResamplingChannelConfig {
    /// The amount of latency added in seconds between the input stream and the
    /// output stream. If this value is too small, then underflows may occur.
    ///
    /// The default value is `0.15` (150 ms).
    pub latency_seconds: f64,

    /// The capacity of the channel in seconds. If this is too small, then
    /// overflows may occur.
    ///
    /// If this value is less than `latency_seconds * 2.0`, then a value of
    /// `latency_seconds * 2.0` will be used instead.
    ///
    /// The default value is `0.4` (400 ms).
    pub capacity_seconds: f64,

    #[cfg(feature = "resampler")]
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
            #[cfg(feature = "resampler")]
            quality: ResampleQuality::Normal,
        }
    }
}

/// Create a new realtime-safe spsc channel for sending samples across streams.
///
/// If the input and output samples rates differ, then this will automatically
/// resample the input stream to match the output stream (unless the "resample"
/// feature is disabled). If the sample rates match, then no resampling will
/// occur.
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
///
/// If the "resampler" feature is disabled, then this will also panic if
/// `in_sample_rate != out_sample_rate`.
pub fn resampling_channel<T: Sample>(
    in_sample_rate: u32,
    out_sample_rate: u32,
    num_channels: usize,
    config: ResamplingChannelConfig,
) -> (ResamplingProd<T>, ResamplingCons<T>) {
    #[cfg(feature = "resampler")]
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
        #[cfg(feature = "resampler")]
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

#[cfg(feature = "resampler")]
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
    #[cfg(feature = "resampler")] resampler: Option<RtResampler<T>>,
    in_sample_rate: u32,
    out_sample_rate: u32,
    num_channels: usize,
    config: ResamplingChannelConfig,
) -> (ResamplingProd<T>, ResamplingCons<T>) {
    #[cfg(not(feature = "resampler"))]
    assert_eq!(
        in_sample_rate, out_sample_rate,
        "Input and output sample rate must be equal when the \"resampler\" feature is disabled."
    );

    assert_ne!(in_sample_rate, 0);
    assert_ne!(out_sample_rate, 0);
    assert_ne!(num_channels, 0);
    assert!(config.latency_seconds > 0.0);
    assert!(config.capacity_seconds > 0.0);

    let in_sample_rate_recip = (in_sample_rate as f64).recip();
    let out_sample_rate_recip = (out_sample_rate as f64).recip();

    let latency_frames = ((in_sample_rate as f64 * config.latency_seconds).round() as usize).max(1);

    let buffer_capacity_frames = ((in_sample_rate as f64 * config.capacity_seconds).round()
        as usize)
        .max(latency_frames * 2);

    let capacity_seconds = buffer_capacity_frames as f64 / in_sample_rate as f64;

    let (mut prod, cons) = ringbuf::HeapRb::<T>::new(buffer_capacity_frames * num_channels).split();

    assert_eq!(prod.capacity().get() % num_channels, 0);

    // Pad the beginning of the buffer with zeros to create the desired latency.
    prod.push_slice(&vec![T::zero(); latency_frames * num_channels]);

    let reset_flag = Arc::new(AtomicBool::new(false));

    let ratio = out_sample_rate as f64 / in_sample_rate as f64;

    (
        ResamplingProd {
            prod,
            num_channels: NonZeroUsize::new(num_channels).unwrap(),
            latency_seconds: config.latency_seconds,
            capacity_seconds,
            in_sample_rate: NonZeroU32::new(in_sample_rate).unwrap(),
            out_sample_rate: NonZeroU32::new(out_sample_rate).unwrap(),
            in_sample_rate_recip,
            reset_flag: Arc::clone(&reset_flag),
        },
        ResamplingCons {
            cons,
            #[cfg(feature = "resampler")]
            resampler,
            num_channels: NonZeroUsize::new(num_channels).unwrap(),
            latency_frames,
            is_waiting_for_frames: true,
            latency_seconds: config.latency_seconds,
            capacity_seconds,
            in_sample_rate: NonZeroU32::new(in_sample_rate).unwrap(),
            out_sample_rate: NonZeroU32::new(out_sample_rate).unwrap(),
            in_sample_rate_recip,
            out_sample_rate_recip,
            ratio,
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
    capacity_seconds: f64,
    in_sample_rate: NonZeroU32,
    out_sample_rate: NonZeroU32,
    in_sample_rate_recip: f64,
    reset_flag: Arc<AtomicBool>,
}

impl<T: Sample + Copy> ResamplingProd<T> {
    /// Push the given data in interleaved format.
    ///
    /// Returns the number of frames (samples in a single channel) that were successfully
    /// pushed. If this number is less than the number of frames in `data`, then it means
    /// an overflow has occured.
    pub fn push_interleaved(&mut self, data: &[T]) -> usize {
        let data_frames = data.len() / self.num_channels.get();

        let pushed_samples = self
            .prod
            .push_slice(&data[..data_frames * self.num_channels.get()]);

        pushed_samples / self.num_channels.get()
    }

    /// Push the given data in de-interleaved format.
    ///
    /// Returns the number of frames (samples in a single channel) that were successfully
    /// pushed. If this number is less than the number of frames in `data`, then it means
    /// an overflow has occured.
    ///
    /// * `range` - The range in each channel in `input` to read data from.
    pub fn push<Vin: AsRef<[T]>>(&mut self, data: &[Vin], range: Range<usize>) -> usize {
        let num_channels = self.num_channels.get();
        let frames = range.end - range.start;

        let (s1, s2) = self.prod.vacant_slices_mut();

        let s1_frames = s1.len() / num_channels;
        debug_assert_eq!(s1_frames * num_channels, s1.len());

        let first_frames = frames.min(s1_frames);

        let s1_part = &mut s1[..first_frames * num_channels];

        // SAFETY:
        //
        // * `&mut [MaybeUninit<T>]` and `&mut [T]` have the same size.
        // * We initialize all data in the slice.
        //
        // TODO: Remove unsafe on `maybe_uninit_write_slice` stabilization.
        unsafe {
            let s1_part = std::mem::transmute::<_, &mut [T]>(s1_part);

            crate::interleave::interleave(data, s1_part, self.num_channels, 0..first_frames);
        }

        let second_frames = if frames > first_frames {
            let s2_frames = s2.len() / num_channels;
            debug_assert_eq!(s2_frames * num_channels, s2.len());

            let second_frames = (frames - first_frames).min(s2_frames);

            let s2_part = &mut s2[..second_frames * num_channels];

            // SAFETY:
            //
            // * `&mut [MaybeUninit<T>]` and `&mut [T]` have the same size.
            // * We initialize all data in the slice.
            //
            // TODO: Remove unsafe on `maybe_uninit_write_slice` stabilization.
            unsafe {
                let s2_part = std::mem::transmute::<_, &mut [T]>(s2_part);

                crate::interleave::interleave(
                    data,
                    s2_part,
                    self.num_channels,
                    first_frames..first_frames + second_frames,
                );
            }

            second_frames
        } else {
            0
        };

        let pushed_frames = first_frames + second_frames;

        // SAFETY:
        //
        // * All vacant data up to `pushed_frames` was initialized above.
        // * This method borrows `self` as mutable, which prevents this
        // producer from being accessed concurrently here.
        unsafe {
            self.prod.advance_write_index(pushed_frames * num_channels);
        }

        pushed_frames
    }

    /// Returns the number of frames (samples in a single channel) that are currently
    /// available to be pushed to the channel.
    pub fn available_frames(&self) -> usize {
        self.prod.vacant_len() / self.num_channels.get()
    }

    /// The amount of data in seconds that is currently available to be pushed to
    /// the channel
    pub fn available_seconds(&self) -> f64 {
        self.available_frames() as f64 * self.in_sample_rate_recip
    }

    /// Returns the number of frames (samples in a single channel) that are currently
    /// occupied in the channel.
    pub fn occupied_frames(&self) -> usize {
        self.prod.occupied_len() / self.num_channels.get()
    }

    /// The amount of data in seconds that is currently occupied in the channel.
    ///
    /// This value will be in the range `[0.0, ResamplingProd::capacity_seconds()]`.
    ///
    /// This value can be used to detect when new packets of data should be pushed
    /// to the channel in a non-realtime thread. Generally, if this value falls below
    /// [`ResamplingProd::latency_seconds()`], then a new packet of data should be
    /// pushed.
    pub fn occupied_seconds(&self) -> f64 {
        self.occupied_frames() as f64 * self.in_sample_rate_recip
    }

    /// The value of [`ResamplingChannelConfig::latency_seconds`] that was passed when
    /// this channel was created.
    pub fn latency_seconds(&self) -> f64 {
        self.latency_seconds
    }

    /// The capacity of the channel in seconds.
    pub fn capacity_seconds(&self) -> f64 {
        self.capacity_seconds
    }

    /// The capacity of the channel in frames (samples in a single channel).
    pub fn capacity_frames(&self) -> usize {
        self.prod.capacity().get() / self.num_channels.get()
    }

    /// The number of channels configured for this stream.
    pub fn num_channels(&self) -> NonZeroUsize {
        self.num_channels
    }

    /// The sample rate of the input (producer) stream.
    pub fn in_sample_rate(&self) -> NonZeroU32 {
        self.in_sample_rate
    }

    /// The sample rate of the output (consumer) stream.
    pub fn out_sample_rate(&self) -> NonZeroU32 {
        self.out_sample_rate
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
    #[cfg(feature = "resampler")]
    resampler: Option<RtResampler<T>>,
    num_channels: NonZeroUsize,
    latency_frames: usize,
    latency_seconds: f64,
    capacity_seconds: f64,
    is_waiting_for_frames: bool,
    in_sample_rate: NonZeroU32,
    out_sample_rate: NonZeroU32,
    in_sample_rate_recip: f64,
    out_sample_rate_recip: f64,
    ratio: f64,
    reset_flag: Arc<AtomicBool>,
}

impl<T: Sample + Copy> ResamplingCons<T> {
    /// The number of channels configured for this stream.
    pub fn num_channels(&self) -> NonZeroUsize {
        self.num_channels
    }

    #[cfg(feature = "resampler")]
    /// Returns `true` if resampling is occurring, `false` if the input and output
    /// sample rates match.
    pub fn is_resampling(&self) -> bool {
        self.resampler.is_some()
    }

    #[cfg(feature = "resampler")]
    /// Get the delay of the internal resampler, reported as a number of output
    /// frames (samples in a single channel).
    ///
    /// If no resampler is active, then this will return `0`.
    pub fn output_delay_frames(&self) -> usize {
        self.resampler
            .as_ref()
            .map(|r| r.output_delay())
            .unwrap_or(0)
    }

    /// The sample rate of the input (producer) stream.
    pub fn in_sample_rate(&self) -> NonZeroU32 {
        self.in_sample_rate
    }

    /// The sample rate of the output (consumer) stream.
    pub fn out_sample_rate(&self) -> NonZeroU32 {
        self.out_sample_rate
    }

    /// The number of output frames (samples in a single channel) in this consumer
    /// that are currently available to be read.
    pub fn available_frames(&self) -> usize {
        let occupied_in_frames = self.occupied_input_frames();

        #[cfg(feature = "resampler")]
        if let Some(resampler) = &self.resampler {
            // The resampler processes in chunks.
            let request_frames = resampler.request_frames();
            let available_in_frames = (occupied_in_frames / request_frames) * request_frames;

            return resampler.temp_frames()
                + (available_in_frames as f64 * self.ratio).floor() as usize;
        }

        occupied_in_frames
    }

    /// The amount of data in seconds that is currently available to read.
    pub fn available_seconds(&self) -> f64 {
        self.available_frames() as f64 * self.out_sample_rate_recip
    }

    /// The amount of data in seconds that is currently occupied in the channel.
    ///
    /// This value will be in the range `[0.0, ResamplingCons::capacity_seconds()]`.
    ///
    /// This can also be used to detect when an extra packet of data should be read or
    /// discarded to correct for jitter.
    pub fn occupied_seconds(&self) -> f64 {
        self.occupied_input_frames() as f64 * self.in_sample_rate_recip
    }

    /// Returns the number of input frames (samples in a single channel) from the producer
    /// (not output frames from this consumer) that are currently occupied in the channel.
    pub fn occupied_input_frames(&self) -> usize {
        self.cons.occupied_len() / self.num_channels.get()
    }

    /// The value of [`ResamplingChannelConfig::latency_seconds`] that was passed when
    /// this channel was created.
    pub fn latency_seconds(&self) -> f64 {
        self.latency_seconds
    }

    /// The capacity of the channel in seconds.
    pub fn capacity_seconds(&self) -> f64 {
        self.capacity_seconds
    }

    /// Clear all queued frames in the buffer.
    pub fn reset(&mut self) {
        #[cfg(feature = "resampler")]
        if let Some(resampler) = &mut self.resampler {
            resampler.reset();
        }

        self.cons.clear();

        self.is_waiting_for_frames = true;
    }

    /// Discard the given number of input frames (samples in a single channel) from the
    /// producer (not output frames from this consumer) from the channel.
    ///
    /// This can be used to correct for jitter and avoid excessive overflows and reduce
    /// perceived audible glitchiness.
    ///
    /// Returns the number of input frames that were discarded.
    pub fn discard_input_frames(&mut self, input_frames: usize) -> usize {
        self.cons
            .skip(input_frames.min(self.occupied_input_frames()) * self.num_channels.get())
            / self.num_channels.get()
    }

    /// If the value of [`ResamplingCons::occupied_seconds()`] is greater than the
    /// given threshold in seconds, then discard the number of input frames needed to
    /// bring the value back down to [`ResamplingCons::latency_seconds()`] to avoid
    /// excessive overflows and reduce perceived audible glitchiness.
    ///
    /// Returns the number of input frames (not output frames) that were discarded.
    ///
    /// If `threshold_seconds` is less than [`ResamplingCons::latency_seconds()`],
    /// then this will do nothing.
    pub fn discard_jitter(&mut self, threshold_seconds: f64) -> usize {
        if threshold_seconds < self.latency_seconds {
            return 0;
        }

        let occupied_seconds = self.occupied_seconds();

        if occupied_seconds >= threshold_seconds || self.cons.vacant_len() == 0 {
            let input_frames = ((occupied_seconds - self.latency_seconds)
                * self.in_sample_rate.get() as f64)
                .round() as usize;
            self.discard_input_frames(input_frames)
        } else {
            0
        }
    }

    /// Read from the channel and store the results into the output buffer
    /// in interleaved format.
    pub fn read_interleaved(&mut self, output: &mut [T]) -> ReadStatus {
        let num_channels = self.num_channels.get();
        let out_frames = output.len() / num_channels;

        if self.reset_flag.swap(false, Ordering::Relaxed) {
            self.reset();
        }

        if self.is_waiting_for_frames {
            if self.available_frames() >= self.latency_frames {
                self.is_waiting_for_frames = false;
            } else {
                output.fill(T::zero());

                return ReadStatus::WaitingForFrames;
            }
        }

        let mut status = ReadStatus::Ok;

        #[cfg(feature = "resampler")]
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

            return status;
        }

        // Simply copy the input stream to the output.

        let samples = self
            .cons
            .pop_slice(&mut output[..out_frames * num_channels]);

        if samples < output.len() {
            status = ReadStatus::Underflow;

            output[samples..].fill(T::zero());

            self.is_waiting_for_frames = true;
        }

        status
    }

    /// Read from the channel and store the results in the output buffer.
    ///
    /// * `output` - The channels of audio data to write data to.
    /// * `range` - The range in each channel in `output` to write data to. Data
    /// inside this range will be fully filled, and any data outside this range
    /// will be untouched.
    pub fn read<Vout: AsMut<[T]>>(
        &mut self,
        output: &mut [Vout],
        range: Range<usize>,
    ) -> ReadStatus {
        let num_channels = self.num_channels.get();

        if output.len() > num_channels {
            for ch in output.iter_mut().skip(num_channels) {
                ch.as_mut()[range.clone()].fill(T::zero());
            }
        }

        if self.reset_flag.swap(false, Ordering::Relaxed) {
            self.reset();
        }

        if self.is_waiting_for_frames {
            if self.available_frames() >= self.latency_frames {
                self.is_waiting_for_frames = false;
            } else {
                for (_, ch) in (0..num_channels).zip(output.iter_mut()) {
                    ch.as_mut()[range.clone()].fill(T::zero());
                }

                return ReadStatus::WaitingForFrames;
            }
        }

        let mut status = ReadStatus::Ok;

        #[cfg(feature = "resampler")]
        if let Some(resampler) = &mut self.resampler {
            resampler.process(
                |in_buf| {
                    // Completely fill the buffer with new data.
                    // If the requested number of samples cannot be appended (i.e.
                    // an underflow occured), then fill the rest with zeros.

                    let request_frames = in_buf[0].len();

                    let (s1, s2) = self.cons.as_slices();

                    let s1_frames = s1.len() / num_channels;
                    debug_assert_eq!(s1_frames * num_channels, s1.len());

                    let first_frames = s1_frames.min(request_frames);

                    crate::interleave::deinterleave(s1, in_buf, self.num_channels, 0..first_frames);

                    let second_frames = if request_frames > first_frames {
                        let s2_frames = s2.len() / num_channels;
                        debug_assert_eq!(s2_frames * num_channels, s2.len());

                        let second_frames = (request_frames - first_frames).min(s2_frames);

                        crate::interleave::deinterleave(
                            s2,
                            in_buf,
                            self.num_channels,
                            first_frames..first_frames + second_frames,
                        );

                        second_frames
                    } else {
                        0
                    };

                    let filled_frames = first_frames + second_frames;

                    // SAFETY:
                    //
                    // * `T` implements `Copy`, meaning it does not implement `Drop` and thus
                    // it is safe to discard.
                    // * This method borrows `self` as mutable, which prevents this
                    // consumer from being accessed concurrently here.
                    unsafe {
                        self.cons.advance_read_index(filled_frames * num_channels);
                    }

                    if filled_frames < request_frames {
                        status = ReadStatus::Underflow;

                        for ch in in_buf.iter_mut() {
                            ch[filled_frames..].fill(T::zero());
                        }

                        self.is_waiting_for_frames = true;
                    }
                },
                output,
                range,
            );

            return status;
        }

        let out_frames = range.end - range.start;

        let (s1, s2) = self.cons.as_slices();

        let s1_frames = s1.len() / num_channels;
        debug_assert_eq!(s1_frames * num_channels, s1.len());

        let first_frames = s1_frames.min(out_frames);

        crate::interleave::deinterleave(
            s1,
            output,
            self.num_channels,
            range.start..range.start + first_frames,
        );

        let second_frames = if out_frames > first_frames {
            let s2_frames = s2.len() / num_channels;
            debug_assert_eq!(s2_frames * num_channels, s2.len());

            let second_frames = (out_frames - first_frames).min(s2_frames);

            crate::interleave::deinterleave(
                s2,
                output,
                self.num_channels,
                range.start + first_frames..range.start + first_frames + second_frames,
            );

            second_frames
        } else {
            0
        };

        let filled_frames = first_frames + second_frames;

        // SAFETY:
        //
        // * `T` implements `Copy`, meaning it does not implement `Drop` and thus
        // it is safe to discard.
        // * This method borrows `self` as mutable, which prevents this
        // consumer from being accessed concurrently here.
        unsafe {
            self.cons.advance_read_index(filled_frames * num_channels);
        }

        if filled_frames < out_frames {
            status = ReadStatus::Underflow;

            for (_, ch) in (0..num_channels).zip(output.iter_mut()) {
                ch.as_mut()[range.start + filled_frames..range.end].fill(T::zero());
            }

            self.is_waiting_for_frames = true;
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
