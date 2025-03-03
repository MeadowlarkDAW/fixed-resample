use std::{
    num::NonZeroUsize,
    ops::Range,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use ringbuf::traits::{Consumer, Observer, Producer, Split};

use crate::Sample;

#[cfg(feature = "resampler")]
use crate::{resampler_type::ResamplerType, FixedResampler, ResampleQuality};

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
    /// Note, the actual capacity may be slightly smaller due to how the internal
    /// sampler processes in chunks.
    ///
    /// The default value is `0.4` (400 ms).
    pub capacity_seconds: f64,

    #[cfg(feature = "resampler")]
    /// The quality of the resampling alrgorithm to use if needed.
    ///
    /// The default value is `ResampleQuality::Normal`.
    pub quality: ResampleQuality,

    #[cfg(feature = "resampler")]
    /// If `true`, then the delay of the internal resampler (if used) will be
    /// subtracted from the `latency_seconds` value to keep the perceived
    /// latency consistent.
    ///
    /// The default value is `true`.
    pub subtract_resampler_delay: bool,
}

impl Default for ResamplingChannelConfig {
    fn default() -> Self {
        Self {
            latency_seconds: 0.15,
            capacity_seconds: 0.4,
            #[cfg(feature = "resampler")]
            quality: ResampleQuality::High,
            #[cfg(feature = "resampler")]
            subtract_resampler_delay: true,
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
/// Internally this uses the `rubato` and `ringbuf` crates.
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
pub fn resampling_channel<T: Sample, const MAX_CHANNELS: usize>(
    num_channels: NonZeroUsize,
    in_sample_rate: u32,
    out_sample_rate: u32,
    config: ResamplingChannelConfig,
) -> (ResamplingProd<T, MAX_CHANNELS>, ResamplingCons<T>) {
    #[cfg(feature = "resampler")]
    let resampler = if in_sample_rate != out_sample_rate {
        Some(FixedResampler::<T, MAX_CHANNELS>::new(
            num_channels,
            in_sample_rate,
            out_sample_rate,
            config.quality,
            true,
        ))
    } else {
        None
    };

    resampling_channel_inner(
        #[cfg(feature = "resampler")]
        resampler,
        num_channels,
        in_sample_rate,
        out_sample_rate,
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
/// Internally this uses the `rubato` and `ringbuf` crates.
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
pub fn resampling_channel_custom<T: Sample, const MAX_CHANNELS: usize>(
    resampler: impl Into<ResamplerType<T>>,
    num_channels: NonZeroUsize,
    in_sample_rate: u32,
    out_sample_rate: u32,
    config: ResamplingChannelConfig,
) -> (ResamplingProd<T, MAX_CHANNELS>, ResamplingCons<T>) {
    let resampler = if in_sample_rate != out_sample_rate {
        let resampler = FixedResampler::<T, MAX_CHANNELS>::from_custom(
            resampler,
            in_sample_rate,
            out_sample_rate,
            true,
        );
        assert_eq!(resampler.num_channels(), num_channels);

        Some(resampler)
    } else {
        None
    };

    resampling_channel_inner(
        resampler,
        num_channels,
        in_sample_rate,
        out_sample_rate,
        config,
    )
}

fn resampling_channel_inner<T: Sample, const MAX_CHANNELS: usize>(
    #[cfg(feature = "resampler")] resampler: Option<FixedResampler<T, MAX_CHANNELS>>,
    num_channels: NonZeroUsize,
    in_sample_rate: u32,
    out_sample_rate: u32,
    config: ResamplingChannelConfig,
) -> (ResamplingProd<T, MAX_CHANNELS>, ResamplingCons<T>) {
    #[cfg(not(feature = "resampler"))]
    assert_eq!(
        in_sample_rate, out_sample_rate,
        "Input and output sample rate must be equal when the \"resampler\" feature is disabled"
    );

    assert_ne!(in_sample_rate, 0);
    assert_ne!(out_sample_rate, 0);
    assert!(config.latency_seconds > 0.0);
    assert!(config.capacity_seconds > 0.0);

    #[cfg(feature = "resampler")]
    let is_resampling = resampler.is_some();
    #[cfg(feature = "resampler")]
    let resampler_output_delay = resampler.as_ref().map(|r| r.output_delay()).unwrap_or(0);

    let out_sample_rate_recip = (out_sample_rate as f64).recip();
    #[cfg(feature = "resampler")]
    let output_to_input_ratio = in_sample_rate as f64 / out_sample_rate as f64;

    let latency_frames =
        ((out_sample_rate as f64 * config.latency_seconds).round() as usize).max(1);

    #[allow(unused_mut)]
    let mut channel_latency_frames = latency_frames;

    #[cfg(feature = "resampler")]
    if resampler.is_some() && config.subtract_resampler_delay {
        if latency_frames > resampler_output_delay {
            channel_latency_frames -= resampler_output_delay;
        } else {
            channel_latency_frames = 0;
        }
    }

    let buffer_capacity_frames = ((in_sample_rate as f64 * config.capacity_seconds).round()
        as usize)
        .max(channel_latency_frames * 2);

    let (mut prod, cons) =
        ringbuf::HeapRb::<T>::new(buffer_capacity_frames * num_channels.get()).split();

    // Fill the channel with initial zeros to create the desired latency.
    prod.push_slice(&vec![T::zero(); channel_latency_frames]);

    let reset_frames_to_discard = Arc::new(AtomicUsize::new(0));

    (
        ResamplingProd {
            prod,
            #[cfg(feature = "resampler")]
            resampler,
            num_channels,
            latency_seconds: config.latency_seconds,
            channel_latency_frames,
            in_sample_rate,
            out_sample_rate,
            out_sample_rate_recip,
            #[cfg(feature = "resampler")]
            output_to_input_ratio,
            reset_frames_to_discard: Arc::clone(&reset_frames_to_discard),
        },
        ResamplingCons {
            cons,
            num_channels,
            latency_frames,
            latency_seconds: config.latency_seconds,
            in_sample_rate,
            out_sample_rate,
            out_sample_rate_recip,
            #[cfg(feature = "resampler")]
            is_resampling,
            #[cfg(feature = "resampler")]
            resampler_output_delay,
            reset_frames_to_discard,
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
/// Internally this uses the `rubato` and `ringbuf` crates.
pub struct ResamplingProd<T: Sample, const MAX_CHANNELS: usize> {
    prod: ringbuf::HeapProd<T>,
    #[cfg(feature = "resampler")]
    resampler: Option<FixedResampler<T, MAX_CHANNELS>>,
    num_channels: NonZeroUsize,
    latency_seconds: f64,
    channel_latency_frames: usize,
    in_sample_rate: u32,
    out_sample_rate: u32,
    out_sample_rate_recip: f64,
    #[cfg(feature = "resampler")]
    output_to_input_ratio: f64,
    reset_frames_to_discard: Arc<AtomicUsize>,
}

impl<T: Sample, const MAX_CHANNELS: usize> ResamplingProd<T, MAX_CHANNELS> {
    /// Push the given data in de-interleaved format.
    ///
    /// * `input` - The input data in de-interleaved format.
    /// * `input_range` - The range in each channel in `input` to read from.
    ///
    /// Returns the number of input frames that were pushed to the channel.
    /// If this value is less than the size of the range, then it means an
    /// underflow has occurred.
    ///
    /// This method is realtime-safe.
    ///
    /// # Panics
    /// Panics if:
    /// * `input.len() < self.num_channels()`.
    /// * The `input_range` is out of bounds for any of the input channels.
    pub fn push<Vin: AsRef<[T]>>(&mut self, input: &[Vin], input_range: Range<usize>) -> usize {
        assert!(input.len() >= self.num_channels.get());

        let input_frames = input_range.end - input_range.start;

        #[cfg(feature = "resampler")]
        if self.resampler.is_some() {
            let available_frames = self.available_frames();

            let proc_frames = input_frames.min(available_frames);

            self.resampler.as_mut().unwrap().process(
                input,
                input_range.start..input_range.start + proc_frames,
                |output_packet| {
                    let packet_frames = output_packet[0].len();

                    let pushed_frames = push_internal(
                        &mut self.prod,
                        &output_packet,
                        0,
                        packet_frames,
                        self.num_channels,
                    );

                    debug_assert_eq!(pushed_frames, packet_frames);
                },
                None,
                true,
            );

            return proc_frames;
        }

        let pushed_frames = push_internal(
            &mut self.prod,
            input,
            input_range.start,
            input_frames,
            self.num_channels,
        );

        pushed_frames
    }

    /// Push the given data in interleaved format.
    ///
    /// Returns the number of input frames that were pushed to the channel.
    /// If this value is less than the size of the range, then it means an
    /// underflow has occurred.
    ///
    /// This method is realtime-safe.
    pub fn push_interleaved(&mut self, input: &[T]) -> usize {
        #[cfg(feature = "resampler")]
        if self.resampler.is_some() {
            let input_frames = input.len() / self.num_channels.get();

            let available_frames = self.available_frames();

            let proc_frames = input_frames.min(available_frames);

            self.resampler.as_mut().unwrap().process_interleaved(
                &input[..proc_frames * self.num_channels.get()],
                |output_packet| {
                    let pushed_samples = self.prod.push_slice(output_packet);

                    debug_assert_eq!(pushed_samples, output_packet.len());
                },
                None,
                true,
            );

            return proc_frames;
        }

        let pushed_samples = self.prod.push_slice(input);

        pushed_samples / self.num_channels.get()
    }

    /// If the channel is empty due to an underflow, fill the channel with
    /// [`ResamplingProd::latency_seconds`] output frames to avoid excessive
    /// underflows in the future and reduce the percieved audible glitchiness.
    ///
    /// This method is realtime-safe.
    pub fn correct_underflows(&mut self) {
        if self.prod.occupied_len() == 0 {
            self.prod
                .push_iter((0..self.channel_latency_frames).map(|_| T::zero()));
        }
    }

    /// Returns the number of input frames (samples in a single channel of audio)
    /// that are currently available to be pushed to the buffer.
    ///
    /// This method is realtime-safe.
    pub fn available_frames(&self) -> usize {
        let output_vacant_frames = self.prod.vacant_len() / self.num_channels.get();

        #[cfg(feature = "resampler")]
        if let Some(resampler) = &self.resampler {
            let mut input_vacant_frames =
                (output_vacant_frames as f64 * self.output_to_input_ratio).floor() as usize;

            // Give some leeway to account for floating point inaccuracies.
            if input_vacant_frames > 0 {
                input_vacant_frames -= 1;
            }

            if input_vacant_frames < resampler.input_block_frames() {
                return 0;
            }

            // The resampler processes in chunks.
            input_vacant_frames = (input_vacant_frames / resampler.input_block_frames())
                * resampler.input_block_frames();

            return input_vacant_frames - resampler.tmp_input_frames();
        }

        output_vacant_frames
    }

    /// The amount of data that is currently present in the channel, in units of
    /// output frames (samples in a single channel of audio).
    ///
    /// Note, this is the number of frames in the *output* audio stream, not the
    /// input audio stream.
    ///
    /// This method is realtime-safe.
    pub fn occupied_output_frames(&self) -> usize {
        self.prod.occupied_len() / self.num_channels.get()
    }

    /// The amount of data that is currently present in the channel, in units of
    /// seconds.
    ///
    /// This method is realtime-safe.
    pub fn occupied_seconds(&self) -> f64 {
        self.occupied_output_frames() as f64 * self.out_sample_rate_recip
    }

    /// The number of channels configured for this stream.
    ///
    /// This method is realtime-safe.
    pub fn num_channels(&self) -> NonZeroUsize {
        self.num_channels
    }

    /// The sample rate of the input stream.
    ///
    /// This method is realtime-safe.
    pub fn in_sample_rate(&self) -> u32 {
        self.in_sample_rate
    }

    /// The sample rate of the output stream.
    ///
    /// This method is realtime-safe.
    pub fn out_sample_rate(&self) -> u32 {
        self.out_sample_rate
    }

    /// The latency of the channel in units of seconds.
    ///
    /// This method is realtime-safe.
    pub fn latency_seconds(&self) -> f64 {
        self.latency_seconds
    }

    /// Returns `true` if this channel is currently resampling.
    ///
    /// This method is realtime-safe.
    #[cfg(feature = "resampler")]
    pub fn is_resampling(&self) -> bool {
        self.resampler.is_some()
    }

    /// Tell the consumer to clear all queued frames in the buffer.
    ///
    /// This method is realtime-safe.
    pub fn reset(&mut self) {
        self.reset_frames_to_discard
            .store(self.occupied_output_frames(), Ordering::Relaxed);

        self.prod
            .push_iter((0..self.channel_latency_frames).map(|_| T::zero()));

        #[cfg(feature = "resampler")]
        if let Some(resampler) = &mut self.resampler {
            resampler.reset();
        }
    }
}

/// The consumer end of a realtime-safe spsc channel for sending samples across
/// streams.
///
/// If the input and output samples rates differ, then this will automatically
/// resample the input stream to match the output stream. If the sample rates
/// match, then no resampling will occur.
///
/// Internally this uses the `rubato` and `ringbuf` crates.
pub struct ResamplingCons<T: Sample> {
    cons: ringbuf::HeapCons<T>,
    num_channels: NonZeroUsize,
    latency_seconds: f64,
    latency_frames: usize,
    #[cfg(feature = "resampler")]
    resampler_output_delay: usize,
    in_sample_rate: u32,
    out_sample_rate: u32,
    out_sample_rate_recip: f64,
    #[cfg(feature = "resampler")]
    is_resampling: bool,
    reset_frames_to_discard: Arc<AtomicUsize>,
}

impl<T: Sample> ResamplingCons<T> {
    /// The number of channels configured for this stream.
    ///
    /// This method is realtime-safe.
    pub fn num_channels(&self) -> NonZeroUsize {
        self.num_channels
    }

    /// The sample rate of the input stream.
    ///
    /// This method is realtime-safe.
    pub fn in_sample_rate(&self) -> u32 {
        self.in_sample_rate
    }

    /// The sample rate of the output stream.
    ///
    /// This method is realtime-safe.
    pub fn out_sample_rate(&self) -> u32 {
        self.out_sample_rate
    }

    /// The latency of the channel in units of seconds.
    ///
    /// This method is realtime-safe.
    pub fn latency_seconds(&self) -> f64 {
        self.latency_seconds
    }

    /// The latency of the channel in units of output frames.
    ///
    /// This method is realtime-safe.
    pub fn latency_frames(&self) -> usize {
        self.latency_frames
    }

    /// The number of frames that are currently available to be read from the
    /// channel.
    ///
    /// This method is realtime-safe.
    pub fn available_frames(&self) -> usize {
        self.cons.occupied_len() / self.num_channels.get()
    }

    /// The amount of data that is currently present in the channel, in units of
    /// seconds.
    ///
    /// This method is realtime-safe.
    pub fn occupied_seconds(&self) -> f64 {
        self.available_frames() as f64 * self.out_sample_rate_recip
    }

    /// Returns `true` if this channel is currently resampling.
    ///
    /// This method is realtime-safe.
    #[cfg(feature = "resampler")]
    pub fn is_resampling(&self) -> bool {
        self.is_resampling
    }

    /// The delay of the internal resampler in number of output frames (samples in
    /// a single channel of audio).
    ///
    /// If there is no resampler being used for this channel, then this will return
    /// `0`.
    ///
    /// This method is realtime-safe.
    #[cfg(feature = "resampler")]
    pub fn resampler_output_delay(&self) -> usize {
        self.resampler_output_delay
    }

    /// Discard a certian number of output frames from the buffer. This can be used
    /// to correct for jitter and avoid excessive overflows and reduce the percieved
    /// audible glitchiness.
    ///
    /// This will discard `frames.min(self.available_frames())` frames.
    ///
    /// Returns the number of output frames that were discarded.
    ///
    /// This method is realtime-safe.
    pub fn discard_frames(&mut self, frames: usize) -> usize {
        self.cons.skip(frames * self.num_channels.get()) / self.num_channels.get()
    }

    /// If the value of [`ResamplingCons::occupied_seconds`] is greater than the
    /// given threshold in seconds, then discard the number of frames needed to
    /// bring the occupied value back to [`ResamplingCons::latency_seconds`] to
    /// avoid excessive overflows in the future and reduce the percieved audible
    /// glitchiness.
    ///
    /// If `threshold_seconds` is less than [`ResamplingCons::latency_seconds`],
    /// then this will do nothing and return 0.
    ///
    /// Returns the number of output frames that were discarded.
    ///
    /// This method is realtime-safe.
    pub fn discard_jitter(&mut self, threshold_seconds: f64) -> usize {
        if threshold_seconds < self.latency_seconds {
            return 0;
        }

        let occupied_seconds = self.occupied_seconds();

        if occupied_seconds > threshold_seconds.max(0.0) {
            let frames = ((threshold_seconds - self.latency_seconds) * self.out_sample_rate as f64)
                .round() as usize;
            self.discard_frames(frames)
        } else {
            0
        }
    }

    /// Read from the channel and store the results in the de-interleaved
    /// output buffer.
    ///
    /// This method is realtime-safe.
    pub fn read<Vout: AsMut<[T]>>(
        &mut self,
        output: &mut [Vout],
        output_range: Range<usize>,
    ) -> ReadStatus {
        let num_channels = self.num_channels.get();

        if output.len() > num_channels {
            for ch in output.iter_mut().skip(num_channels) {
                ch.as_mut()[output_range.clone()].fill(T::zero());
            }
        }

        let frames_to_discard = self.reset_frames_to_discard.swap(0, Ordering::Relaxed);
        if frames_to_discard > 0 {
            self.discard_frames(frames_to_discard);
        }

        let output_frames = output_range.end - output_range.start;

        // Simply copy the input stream to the output.
        let (s1, s2) = self.cons.as_slices();

        let s1_frames = s1.len() / num_channels;
        let s1_copy_frames = s1_frames.min(output_frames);

        fast_interleave::deinterleave_variable(
            s1,
            self.num_channels,
            output,
            output_range.start..output_range.start + s1_copy_frames,
        );

        let mut filled_frames = s1_copy_frames;

        if output_frames > s1_copy_frames {
            let s2_frames = s2.len() / num_channels;
            let s2_copy_frames = s2_frames.min(output_frames - s1_copy_frames);

            fast_interleave::deinterleave_variable(
                s2,
                self.num_channels,
                output,
                output_range.start + s1_copy_frames
                    ..output_range.start + s1_copy_frames + s2_copy_frames,
            );

            filled_frames += s2_copy_frames;
        }

        // SAFETY:
        //
        // * `T` implements `Copy`, so it does not have a drop method that needs to
        // be called.
        // * `self` is borrowed as mutable in this method, ensuring that the consumer
        // cannot be accessed concurrently.
        unsafe {
            self.cons
                .advance_read_index(filled_frames * self.num_channels.get());
        }

        if filled_frames < output_frames {
            for (_, ch) in (0..num_channels).zip(output.iter_mut()) {
                ch.as_mut()[filled_frames..output_range.end].fill(T::zero());
            }

            ReadStatus::Underflow {
                num_frames_dropped: output_frames - filled_frames,
            }
        } else {
            ReadStatus::Ok
        }
    }

    /// Read from the channel and store the results into the output buffer
    /// in interleaved format.
    ///
    /// This method is realtime-safe.
    pub fn read_interleaved(&mut self, output: &mut [T]) -> ReadStatus {
        let num_channels = self.num_channels.get();
        let out_frames = output.len() / num_channels;

        let frames_to_discard = self.reset_frames_to_discard.swap(0, Ordering::Relaxed);
        if frames_to_discard > 0 {
            self.discard_frames(frames_to_discard);
        }

        let pushed_samples = self
            .cons
            .pop_slice(&mut output[..out_frames * num_channels]);

        if pushed_samples < output.len() {
            output[pushed_samples..].fill(T::zero());

            ReadStatus::Underflow {
                num_frames_dropped: out_frames - (pushed_samples / self.num_channels.get()),
            }
        } else {
            ReadStatus::Ok
        }
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
    Underflow { num_frames_dropped: usize },
}

fn push_internal<T: Sample, Vin: AsRef<[T]>>(
    prod: &mut ringbuf::HeapProd<T>,
    input: &[Vin],
    in_start_frame: usize,
    frames: usize,
    num_channels: NonZeroUsize,
) -> usize {
    let (s1, s2) = prod.vacant_slices_mut();

    if s1.len() == 0 {
        return 0;
    }

    let s1_frames = s1.len() / num_channels.get();
    let s1_copy_frames = s1_frames.min(frames);

    let mut frames_pushed = s1_copy_frames;

    {
        // SAFETY:
        //
        // * `&mut [MaybeUninit<T>]` and `&mut [T]` are the same bit-for-bit.
        // * All data in the slice is initialized in the `interleave` method below.
        //
        // TODO: Remove unsafe on `maybe_uninit_write_slice` stabilization.
        let s1: &mut [T] =
            unsafe { std::mem::transmute(&mut s1[..s1_copy_frames * num_channels.get()]) };

        fast_interleave::interleave_variable(
            input,
            in_start_frame..in_start_frame + s1_copy_frames,
            s1,
            num_channels,
        );
    }

    if frames > s1_copy_frames && s2.len() > 0 {
        let s2_frames = s2.len() / num_channels.get();
        let s2_copy_frames = s2_frames.min(frames - s1_copy_frames);

        // SAFETY:
        //
        // * `&mut [MaybeUninit<T>]` and `&mut [T]` are the same bit-for-bit.
        // * All data in the slice is initialized in the `interleave` method below.
        //
        // TODO: Remove unsafe on `maybe_uninit_write_slice` stabilization.
        let s2: &mut [T] =
            unsafe { std::mem::transmute(&mut s2[..s2_copy_frames * num_channels.get()]) };

        fast_interleave::interleave_variable(
            input,
            in_start_frame + s1_copy_frames..in_start_frame + s1_copy_frames + s2_copy_frames,
            s2,
            num_channels,
        );

        frames_pushed += s2_copy_frames;
    }

    // SAFETY:
    //
    // * All frames up to `frames_pushed` was filled with data above.
    // * `prod` is borrowed as mutable in this method, ensuring that it cannot be
    // accessed concurrently.
    unsafe {
        prod.advance_write_index(frames_pushed * num_channels.get());
    }

    frames_pushed
}
