use std::{num::NonZeroUsize, ops::Range};

use arrayvec::ArrayVec;
use rubato::Sample;

use crate::resampler_type::ResamplerType;

/// The quality of the resampling algorithm used for a [`FixedResampler`].
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResampleQuality {
    /// Decent quality, fast performance, low latency
    ///
    /// This is recommended for most realtime applications where low latency
    /// is desired.
    ///
    /// Internally this uses the [`FastFixedIn`] resampler from rubato with
    /// linear interpolation.
    Low,
    #[default]
    /// Great quality, medium performance, high latency
    ///
    /// This is recommended for most non-realtime applications where higher
    /// latency is not an issue.
    ///
    /// Note, this resampler type adds a significant amount of latency (in
    /// the hundreds of frames), so prefer to use the "Low" option if low
    /// latency is desired.
    ///
    /// If the `fft-resampler` feature is not enabled, then this will fall
    /// back to "Low".
    ///
    /// Internally this uses the [`FftFixedIn`] resampler from rubato with
    /// a chunk size of `1024` and `2` sub-chunks.
    High,
}

/// Options for processes the last packet in a resampler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LastPacketInfo {
    /// The desired number of output frames that should be sent via the
    /// `on_output_packet` closure.
    ///
    /// If this is `None`, then the last packet sent may contain extra
    /// padded zeros on the end.
    pub desired_output_frames: Option<u64>,
}

/// An easy-to-use resampler with a fixed ratio.
///
/// Internally this uses the `rubato` crate.
pub struct FixedResampler<T: Sample, const MAX_CHANNELS: usize> {
    resampler: ResamplerType<T>,
    tmp_deintlv_in_buf: Vec<T>,
    tmp_deintlv_out_buf: Vec<T>,
    tmp_intlv_buf: Vec<T>,
    tmp_deintlv_in_buf_len: usize,
    num_channels: NonZeroUsize,
    input_block_frames: usize,
    max_output_block_frames: usize,
    output_delay: usize,
    delay_frames_left: usize,
    in_sample_rate: u32,
    out_sample_rate: u32,
    interleaved: bool,
}

impl<T: Sample, const MAX_CHANNELS: usize> FixedResampler<T, MAX_CHANNELS> {
    /// Create a new [`FixedResampler`].
    ///
    /// * `num_channels` - The number of audio channels.
    /// * `in_sample_rate` - The sample rate of the input data.
    /// * `out_sample_rate` - The sample rate of the output data.
    /// * `quality` - The quality of the resampling algorithm to use.
    /// * `interleaved` - If you plan on using [`FixedResampler::process_interleaved`],
    /// then set this to `true`. Otherwise, you can set this to `false` to
    /// save a bit of memory.
    ///
    /// # Panics
    /// Panics if:
    /// * `num_channels == 0`
    /// * `num_channels > MAX_CHANNELS`
    /// * `in_sample_rate == 0`
    /// * `out_sample_rate == 0`
    pub fn new(
        num_channels: NonZeroUsize,
        in_sample_rate: u32,
        out_sample_rate: u32,
        quality: ResampleQuality,
        interleaved: bool,
    ) -> Self {
        Self::from_custom(
            ResamplerType::from_quality(
                in_sample_rate,
                out_sample_rate,
                num_channels,
                quality.into(),
            ),
            in_sample_rate,
            out_sample_rate,
            interleaved,
        )
    }

    /// Create a new [`FixedResampler`] using the custom resampler.
    ///
    /// * `resampler` - The resampler to use.
    /// * `in_sample_rate` - The sample rate of the input data.
    /// * `out_sample_rate` - The sample rate of the output data.
    /// * `interleaved` - If you plan on using [`FixedResampler::process_interleaved`],
    /// then set this to `true`. Otherwise, you can set this to `false` to
    /// save a bit of memory.
    ///
    /// # Panics
    /// Panics if:
    /// * `resampler.num_channels() == 0`
    /// * `resampler.num_channels() == 0 > MAX_CHANNELS`
    /// * `in_sample_rate == 0`
    /// * `out_sample_rate == 0`
    pub fn from_custom(
        resampler: impl Into<ResamplerType<T>>,
        in_sample_rate: u32,
        out_sample_rate: u32,
        interleaved: bool,
    ) -> Self {
        assert_ne!(in_sample_rate, 0);
        assert_ne!(out_sample_rate, 0);

        let mut resampler: ResamplerType<T> = resampler.into();

        let num_channels = NonZeroUsize::new(resampler.num_channels()).unwrap();

        assert!(num_channels.get() <= MAX_CHANNELS);

        let input_block_frames = resampler.input_frames_max();
        let max_output_block_frames = resampler.output_frames_max();
        let output_delay = resampler.output_delay();

        let tmp_in_buf_len = input_block_frames * num_channels.get();
        let tmp_out_buf_len = max_output_block_frames * num_channels.get();

        let mut tmp_deintlv_in_buf = Vec::new();
        tmp_deintlv_in_buf.reserve_exact(tmp_in_buf_len);
        tmp_deintlv_in_buf.resize(tmp_in_buf_len, T::zero());

        let mut tmp_deintlv_out_buf = Vec::new();
        tmp_deintlv_out_buf.reserve_exact(tmp_out_buf_len);
        tmp_deintlv_out_buf.resize(tmp_out_buf_len, T::zero());

        let tmp_intlv_buf = if interleaved && num_channels.get() > 1 {
            let intlv_buf_len =
                input_block_frames.max(max_output_block_frames) * num_channels.get();
            let mut v = Vec::new();
            v.reserve_exact(intlv_buf_len);
            v.resize(intlv_buf_len, T::zero());
            v
        } else {
            Vec::new()
        };

        Self {
            resampler,
            tmp_deintlv_in_buf,
            tmp_deintlv_out_buf,
            tmp_intlv_buf,
            tmp_deintlv_in_buf_len: 0,
            num_channels,
            input_block_frames,
            max_output_block_frames,
            output_delay,
            delay_frames_left: output_delay,
            in_sample_rate,
            out_sample_rate,
            interleaved,
        }
    }

    /// The number of channels configured for this resampler.
    pub fn num_channels(&self) -> NonZeroUsize {
        self.num_channels
    }

    /// The input sample rate configured for this resampler.
    pub fn in_sample_rate(&self) -> u32 {
        self.in_sample_rate
    }

    /// The output sample rate configured for this resampler.
    pub fn out_sample_rate(&self) -> u32 {
        self.out_sample_rate
    }

    /// The number of frames (samples in a single channel of audio) that appear in
    /// a single packet of input data in the internal resampler.
    pub fn input_block_frames(&self) -> usize {
        self.input_block_frames
    }

    /// The maximum number of frames (samples in a single channel of audio) that can
    /// appear in a single call to the `on_output_packet` closure in
    /// [`FixedResampler::process`] and [`FixedResampler::process_interleaved`].
    pub fn max_output_block_frames(&self) -> usize {
        self.max_output_block_frames
    }

    /// The delay introduced by the internal resampler in number of output frames (
    /// samples in a single channel of audio).
    pub fn output_delay(&self) -> usize {
        self.output_delay
    }

    /// Whether or not the `interleaved` argument was set to `true` in the constructor.
    pub fn is_interleaved(&self) -> bool {
        self.interleaved
    }

    /// The number of frames (samples in a single channel of audio) that are needed
    /// for an output buffer given the number of input frames.
    pub fn out_alloc_frames(&self, input_frames: u64) -> u64 {
        ((input_frames * self.out_sample_rate as u64) / self.in_sample_rate as u64) + 1
    }

    #[allow(unused)]
    pub(crate) fn tmp_input_frames(&self) -> usize {
        self.tmp_deintlv_in_buf_len
    }

    /// Process the given de-interleaved input data and return packets of de-interleaved
    /// resampled output data.
    ///
    /// * `input` - The de-interleaved channels of input data.
    /// * `input_range` - The range in each input channel to read from.
    /// * `on_output_packet` - Gets called whenever there is a new packet of resampled
    /// output data. The output data is in de-interleaved format.
    /// * `last_packet` - If this is `Some`, then any leftover input samples in the
    /// buffer will be flushed out and the resampler reset. Use this if this is the
    /// last/only packet of input data when used in a non-realtime context.
    /// * `trim_delay` - If `true`, then the initial padded zeros introduced by the
    /// internal resampler will be trimmed off. If this is being used in a realtime
    /// context, then prefer to set this to `false`.
    ///
    /// This method is realtime-safe.
    ///
    /// # Panics
    /// Panics if:
    /// * `input.len() < self.num_channels()`
    /// * The `input_range` is out of bounds for any of the input channels.
    pub fn process<Vin: AsRef<[T]>>(
        &mut self,
        input: &[Vin],
        input_range: Range<usize>,
        mut on_output_packet: impl FnMut(ArrayVec<&[T], MAX_CHANNELS>),
        last_packet: Option<LastPacketInfo>,
        trim_delay: bool,
    ) {
        assert!(input.len() >= self.num_channels.get());

        {
            let mut on_output_packet_inner =
                move |output_packet: ArrayVec<&[T], MAX_CHANNELS>, _tmp_intlv_buf: &mut Vec<T>| {
                    (on_output_packet)(output_packet);
                };

            let total_input_frames = input_range.end - input_range.start;

            let mut tmp_deintlv_in_buf_slices: ArrayVec<&mut [T], MAX_CHANNELS> = self
                .tmp_deintlv_in_buf
                .chunks_exact_mut(self.input_block_frames)
                .collect();
            let mut tmp_deintlv_out_buf_slices: ArrayVec<&mut [T], MAX_CHANNELS> = self
                .tmp_deintlv_out_buf
                .chunks_exact_mut(self.max_output_block_frames)
                .collect();

            let mut input_frames_processed = 0;
            let mut output_frames_processed = 0;

            let desired_output_frames = last_packet.and_then(|info| info.desired_output_frames);

            while input_frames_processed < total_input_frames {
                if self.tmp_deintlv_in_buf_len == 0
                    && (total_input_frames - input_frames_processed) >= self.input_block_frames
                {
                    // We can use the input data directly to avoid an extra copy.

                    let input_slices: ArrayVec<&[T], MAX_CHANNELS> = input
                        [..self.num_channels.get()]
                        .iter()
                        .map(|s| {
                            &s.as_ref()[input_range.start + input_frames_processed
                                ..input_range.start
                                    + input_frames_processed
                                    + self.input_block_frames]
                        })
                        .collect();

                    resample_inner(
                        &mut self.resampler,
                        &input_slices,
                        &mut tmp_deintlv_out_buf_slices,
                        &mut on_output_packet_inner,
                        &mut output_frames_processed,
                        desired_output_frames,
                        &mut self.delay_frames_left,
                        trim_delay,
                        &mut self.tmp_intlv_buf,
                    );

                    input_frames_processed += self.input_block_frames;
                } else {
                    let copy_frames = (self.input_block_frames - self.tmp_deintlv_in_buf_len)
                        .min(total_input_frames - input_frames_processed);

                    for (in_slice_ch, input_ch) in
                        tmp_deintlv_in_buf_slices.iter_mut().zip(input.iter())
                    {
                        in_slice_ch[self.tmp_deintlv_in_buf_len
                            ..self.tmp_deintlv_in_buf_len + copy_frames]
                            .copy_from_slice(
                                &input_ch.as_ref()[input_range.start + input_frames_processed
                                    ..input_range.start + input_frames_processed + copy_frames],
                            );
                    }

                    self.tmp_deintlv_in_buf_len += copy_frames;
                    input_frames_processed += copy_frames;

                    if self.tmp_deintlv_in_buf_len < self.input_block_frames {
                        // Must wait for more data before resampling the next packet.
                        break;
                    }

                    resample_inner(
                        &mut self.resampler,
                        &tmp_deintlv_in_buf_slices,
                        &mut tmp_deintlv_out_buf_slices,
                        &mut on_output_packet_inner,
                        &mut output_frames_processed,
                        desired_output_frames,
                        &mut self.delay_frames_left,
                        trim_delay,
                        &mut self.tmp_intlv_buf,
                    );

                    self.tmp_deintlv_in_buf_len = 0;
                }
            }

            if last_packet.is_some() {
                process_last_packet(
                    &mut tmp_deintlv_in_buf_slices,
                    &mut tmp_deintlv_out_buf_slices,
                    &mut self.resampler,
                    &mut on_output_packet_inner,
                    &mut output_frames_processed,
                    desired_output_frames,
                    &mut self.delay_frames_left,
                    trim_delay,
                    &mut self.tmp_intlv_buf,
                    self.tmp_deintlv_in_buf_len,
                );
            }
        }

        if last_packet.is_some() {
            self.reset();
        }
    }

    /// Process the given interleaved input data and return packets of interleaved
    /// resampled output data.
    ///
    /// * `input` - The interleaved input data.
    /// * `on_output_packet` - Gets called whenever there is a new packet of resampled
    /// output data. The output data is in interleaved format.
    /// * `last_packet` - If this is `Some`, then any leftover input samples in the
    /// buffer will be flushed out and the resampler reset. Use this if this is the
    /// last/only packet of input data when used in a non-realtime context.
    /// * `trim_delay` - If `true`, then the initial padded zeros introduced by the
    /// internal resampler will be trimmed off. If this is being used in a realtime
    /// context, then prefer to set this to `false`.
    ///
    /// This method is realtime-safe.
    ///
    /// # Panics
    /// Panics if the `interleaved` argument in the constructor was `false`.
    pub fn process_interleaved(
        &mut self,
        input: &[T],
        mut on_output_packet: impl FnMut(&[T]),
        last_packet: Option<LastPacketInfo>,
        trim_delay: bool,
    ) {
        assert!(self.interleaved, "The constructor argument \"interleaved\" must be set to \"true\" in order to call FixedResampler::process_interleaved");

        {
            let num_channels = self.num_channels;

            let mut on_output_packet_inner =
                move |output_packet: ArrayVec<&[T], MAX_CHANNELS>, tmp_intlv_buf: &mut Vec<T>| {
                    let frames = output_packet[0].len();

                    if num_channels.get() == 1 {
                        (on_output_packet)(&output_packet[0]);
                    } else {
                        fast_interleave::interleave_variable(
                            &output_packet,
                            0..frames,
                            tmp_intlv_buf.as_mut_slice(),
                            num_channels,
                        );

                        (on_output_packet)(&tmp_intlv_buf[..frames * num_channels.get()]);
                    }
                };

            let total_input_frames = input.len() / self.num_channels;

            let mut tmp_deintlv_in_buf_slices: ArrayVec<&mut [T], MAX_CHANNELS> = self
                .tmp_deintlv_in_buf
                .chunks_exact_mut(self.input_block_frames)
                .collect();
            let mut tmp_deintlv_out_buf_slices: ArrayVec<&mut [T], MAX_CHANNELS> = self
                .tmp_deintlv_out_buf
                .chunks_exact_mut(self.max_output_block_frames)
                .collect();

            let mut input_frames_processed = 0;
            let mut output_frames_processed = 0;

            let desired_output_frames = last_packet.and_then(|info| info.desired_output_frames);

            while input_frames_processed < total_input_frames {
                let copy_frames = (self.input_block_frames - self.tmp_deintlv_in_buf_len)
                    .min(total_input_frames - input_frames_processed);

                fast_interleave::deinterleave_variable(
                    &input[input_frames_processed * self.num_channels.get()
                        ..(input_frames_processed + copy_frames) * self.num_channels.get()],
                    self.num_channels,
                    &mut tmp_deintlv_in_buf_slices,
                    self.tmp_deintlv_in_buf_len..self.tmp_deintlv_in_buf_len + copy_frames,
                );

                self.tmp_deintlv_in_buf_len += copy_frames;
                input_frames_processed += copy_frames;

                if self.tmp_deintlv_in_buf_len < self.input_block_frames {
                    // Must wait for more data before resampling the next packet.
                    break;
                }

                resample_inner(
                    &mut self.resampler,
                    &tmp_deintlv_in_buf_slices,
                    &mut tmp_deintlv_out_buf_slices,
                    &mut on_output_packet_inner,
                    &mut output_frames_processed,
                    desired_output_frames,
                    &mut self.delay_frames_left,
                    trim_delay,
                    &mut self.tmp_intlv_buf,
                );

                self.tmp_deintlv_in_buf_len = 0;
            }

            if last_packet.is_some() {
                process_last_packet(
                    &mut tmp_deintlv_in_buf_slices,
                    &mut tmp_deintlv_out_buf_slices,
                    &mut self.resampler,
                    &mut on_output_packet_inner,
                    &mut output_frames_processed,
                    desired_output_frames,
                    &mut self.delay_frames_left,
                    trim_delay,
                    &mut self.tmp_intlv_buf,
                    self.tmp_deintlv_in_buf_len,
                );
            }
        }

        if last_packet.is_some() {
            self.reset();
        }
    }

    /// Reset the state of the resampler.
    ///
    /// This method is realtime-safe.
    pub fn reset(&mut self) {
        self.resampler.reset();
        self.tmp_deintlv_in_buf_len = 0;
        self.delay_frames_left = self.output_delay;
    }
}

impl<T: Sample, const MAX_CHANNELS: usize> Into<ResamplerType<T>>
    for FixedResampler<T, MAX_CHANNELS>
{
    fn into(self) -> ResamplerType<T> {
        self.resampler
    }
}

fn process_last_packet<T: Sample, const MAX_CHANNELS: usize>(
    tmp_deintlv_in_buf_slices: &mut ArrayVec<&mut [T], MAX_CHANNELS>,
    tmp_deintlv_out_buf_slices: &mut ArrayVec<&mut [T], MAX_CHANNELS>,
    resampler: &mut ResamplerType<T>,
    on_output_packet: &mut impl FnMut(ArrayVec<&[T], MAX_CHANNELS>, &mut Vec<T>),
    output_frames_processed: &mut u64,
    desired_output_frames: Option<u64>,
    delay_frames_left: &mut usize,
    trim_delay: bool,
    tmp_intlv_buf: &mut Vec<T>,
    tmp_deintlv_in_buf_len: usize,
) {
    if tmp_deintlv_in_buf_len > 0 {
        for ch in tmp_deintlv_in_buf_slices.iter_mut() {
            ch[tmp_deintlv_in_buf_len..].fill(T::zero());
        }

        resample_inner(
            resampler,
            &tmp_deintlv_in_buf_slices,
            tmp_deintlv_out_buf_slices,
            on_output_packet,
            output_frames_processed,
            desired_output_frames,
            delay_frames_left,
            trim_delay,
            tmp_intlv_buf,
        );
    }

    for ch in tmp_deintlv_in_buf_slices.iter_mut() {
        ch.fill(T::zero());
    }

    if let Some(desired_output_frames) = desired_output_frames {
        if *output_frames_processed >= desired_output_frames {
            return;
        }

        while *output_frames_processed < desired_output_frames {
            resample_inner(
                resampler,
                &tmp_deintlv_in_buf_slices,
                tmp_deintlv_out_buf_slices,
                on_output_packet,
                output_frames_processed,
                Some(desired_output_frames),
                delay_frames_left,
                trim_delay,
                tmp_intlv_buf,
            );
        }
    } else {
        resample_inner(
            resampler,
            &tmp_deintlv_in_buf_slices,
            tmp_deintlv_out_buf_slices,
            on_output_packet,
            output_frames_processed,
            desired_output_frames,
            delay_frames_left,
            trim_delay,
            tmp_intlv_buf,
        );
    }
}

fn resample_inner<T: Sample, Vin: AsRef<[T]>, const MAX_CHANNELS: usize>(
    resampler: &mut ResamplerType<T>,
    input: &[Vin],
    tmp_deintlv_out_buf_slices: &mut ArrayVec<&mut [T], MAX_CHANNELS>,
    on_output_packet: &mut impl FnMut(ArrayVec<&[T], MAX_CHANNELS>, &mut Vec<T>),
    output_frames_processed: &mut u64,
    desired_output_frames: Option<u64>,
    delay_frames_left: &mut usize,
    trim_delay: bool,
    tmp_intlv_buf: &mut Vec<T>,
) {
    let (_, output_frames) = resampler
        .process_into_buffer(input, tmp_deintlv_out_buf_slices, None)
        .unwrap();

    let (output_packet_start, mut packet_output_frames) = if trim_delay && *delay_frames_left > 0 {
        let delay_frames = output_frames.min(*delay_frames_left);
        *delay_frames_left -= delay_frames;
        (delay_frames, output_frames - delay_frames)
    } else {
        (0, output_frames)
    };

    if let Some(desired_output_frames) = desired_output_frames {
        if desired_output_frames <= *output_frames_processed {
            packet_output_frames = 0;
        } else if (desired_output_frames - *output_frames_processed) < packet_output_frames as u64 {
            packet_output_frames = (desired_output_frames - *output_frames_processed) as usize
        }
    }

    if packet_output_frames > 0 {
        let out_packet_slices: ArrayVec<&[T], MAX_CHANNELS> = tmp_deintlv_out_buf_slices
            .iter()
            .map(|s| &s[output_packet_start..output_packet_start + packet_output_frames])
            .collect();

        (on_output_packet)(out_packet_slices, tmp_intlv_buf);
    }

    *output_frames_processed += packet_output_frames as u64;
}
