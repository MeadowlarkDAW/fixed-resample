use std::num::NonZeroUsize;

use rubato::Sample;

use crate::{ResampleQuality, ResamplerType};

/// An easy to use resampler for use in non-realtime applications.
pub struct NonRtResampler<T: Sample> {
    resampler: ResamplerType<T>,
    in_buf: Vec<Vec<T>>,
    out_buf: Vec<Vec<T>>,
    intlv_buf: Vec<T>,
    in_buf_len: usize,
    num_channels: NonZeroUsize,
    input_frames_max: usize,
    output_frames_max: usize,
    output_delay: usize,
    delay_frames_left: usize,
}

impl<T: Sample> NonRtResampler<T> {
    /// Create a new non-realtime resampler.
    ///
    /// * `in_sample_rate` - The sample rate of the input stream.
    /// * `out_sample_rate` - The sample rate of the output stream.
    /// * `num_channels` - The number of channels.
    /// * `interleaved` - If you are planning to use [`NonRtResampler::process_interleaved`],
    /// set this to `true`. Otherwise you can set this to `false` to save a bit of
    /// memory.
    /// * `quality` - The quality of the resampling algorithm.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// * `in_sample_rate == 0`
    /// * `out_sample_rate == 0`
    /// * `num_channels == 0`,
    pub fn new(
        in_sample_rate: u32,
        out_sample_rate: u32,
        num_channels: usize,
        quality: ResampleQuality,
    ) -> Self {
        let resampler =
            ResamplerType::from_quality(in_sample_rate, out_sample_rate, num_channels, quality);
        Self::from_custom(resampler)
    }

    /// Create a new non-realtime resampler using the given rubato resampler.
    ///
    /// # Panics
    /// Panics if `max_out_frames == 0`.
    pub fn from_custom(resampler: impl Into<ResamplerType<T>>) -> Self {
        let mut resampler: ResamplerType<T> = resampler.into();

        let num_channels = resampler.num_channels();
        assert!(num_channels > 0);

        let input_frames_max = resampler.input_frames_max();
        let output_frames_max = resampler.output_frames_max();
        let output_delay = resampler.output_delay();

        Self {
            resampler,
            in_buf: (0..num_channels)
                .map(|_| vec![T::zero(); input_frames_max])
                .collect(),
            out_buf: (0..num_channels)
                .map(|_| vec![T::zero(); output_frames_max])
                .collect(),
            intlv_buf: Vec::new(),
            in_buf_len: 0,
            num_channels: NonZeroUsize::new(num_channels).unwrap(),
            input_frames_max,
            output_frames_max,
            output_delay,
            delay_frames_left: output_delay,
        }
    }

    /// Get the number of channels this resampler is configured for.
    pub fn num_channels(&self) -> NonZeroUsize {
        self.num_channels
    }

    /// Reset the resampler state and clear all internal buffers.
    pub fn reset(&mut self) {
        self.resampler.reset();
        self.in_buf_len = 0;
        self.delay_frames_left = self.output_delay;
    }

    /// Get the delay of the internal resampler, reported as a number of output frames.
    ///
    /// Note, this resampler automatically truncates the starting padded zero frames for
    /// you.
    pub fn output_delay(&self) -> usize {
        self.output_delay
    }

    /// The maximum number of frames that can appear in a single call to the closure in
    /// [`NonRtResampler::process()`] and [`NonRtResampler::process_interleaved()`].
    pub fn max_processed_frames(&self) -> usize {
        self.output_frames_max
    }

    /// The length of an output buffer in frames for a given input buffer.
    ///
    /// Note, the resampler may add extra padded zeros to the end of the output buffer,
    /// so prefer to use [`NonRtResampler::out_alloc_frames()`] instead to reserve the
    /// capacity for an output buffer.
    pub fn out_frames(&self, in_sample_rate: u32, out_sample_rate: u32, in_frames: usize) -> usize {
        (in_frames as f64 * out_sample_rate as f64 / in_sample_rate as f64).ceil() as usize
    }

    /// The number of frames needed to allocate an output buffer for a given input buffer.
    pub fn out_alloc_frames(
        &self,
        in_sample_rate: u32,
        out_sample_rate: u32,
        in_frames: usize,
    ) -> usize {
        self.out_frames(in_sample_rate, out_sample_rate, in_frames)
            + (self.max_processed_frames() * 2)
    }

    /// Resample the given input data.
    ///
    /// * `input` - The input data.
    /// * `on_processed` - Called whenever there is new resampled data. The first argument
    /// is the buffers containing the new data, and the second argument is the length of
    /// the new data in frames (NOTE, the second argument may be less than the length of
    /// the `Vec`s in the first argument).
    /// * `is_last_packet` - Whether or not this is the last (or only) packet of data that
    /// will be resampled. This ensures that any leftover samples in the internal resampler
    /// are flushed to the output.
    ///
    /// If you want to flush the remaining samples out of the internal resampler when there
    /// is no more input data left, set the `input` to an empty slice with no channels
    /// (`&[]`), and set `is_last_packet` to `true`.
    ///
    /// Note, when flushing the remaining data with `is_last_packet`, the resulting output
    /// may have a few extra padded zero samples on the end.
    ///
    /// The delay of the internal resampler is automatically accounted for (the starting
    /// padded zero frames are automatically truncated). Call [`NonRtResampler::reset()`]
    /// before reusing this resampler for a new sample.
    ///
    /// Note, this method is *NOT* realtime-safe.
    pub fn process<Vin: AsRef<[T]>>(
        &mut self,
        input: &[Vin],
        mut on_processed: impl FnMut(&[Vec<T>], usize),
        is_last_packet: bool,
    ) {
        let mut resample = |self_: &mut Self| {
            let (_, out_frames) = self_
                .resampler
                .process_into_buffer(&self_.in_buf, &mut self_.out_buf, None)
                .unwrap();

            if self_.delay_frames_left == 0 {
                (on_processed)(self_.out_buf.as_slice(), out_frames);
            } else if self_.delay_frames_left < out_frames {
                for b in self_.out_buf.iter_mut() {
                    b.copy_within(self_.delay_frames_left..out_frames, 0);
                }

                (on_processed)(
                    self_.out_buf.as_slice(),
                    out_frames - self_.delay_frames_left,
                );

                self_.delay_frames_left = 0;
            } else {
                self_.delay_frames_left -= out_frames;
            }
        };

        if input.is_empty() && is_last_packet {
            for b in self.in_buf.iter_mut() {
                b.fill(T::zero());
            }

            resample(self);

            return;
        }

        let in_ch_0 = input[0].as_ref();
        let total_in_frames = in_ch_0.len();

        let mut in_frames_copied = 0;
        while in_frames_copied < total_in_frames {
            if self.in_buf_len < self.input_frames_max {
                let copy_frames = (self.input_frames_max - self.in_buf_len)
                    .min(total_in_frames - in_frames_copied);

                self.in_buf[0][self.in_buf_len..self.in_buf_len + copy_frames]
                    .copy_from_slice(&in_ch_0[in_frames_copied..in_frames_copied + copy_frames]);
                for (in_buf_ch, in_ch) in self.in_buf.iter_mut().zip(input.iter()).skip(1) {
                    let in_ch = in_ch.as_ref();
                    in_buf_ch[self.in_buf_len..self.in_buf_len + copy_frames]
                        .copy_from_slice(&in_ch[in_frames_copied..in_frames_copied + copy_frames]);
                }

                self.in_buf_len += copy_frames;
                in_frames_copied += copy_frames;

                if self.in_buf_len < self.input_frames_max && !is_last_packet {
                    // Must wait for more input data before continuing.
                    return;
                }
            }

            if is_last_packet && in_frames_copied == total_in_frames {
                for ch in self.in_buf.iter_mut() {
                    ch[self.in_buf_len..].fill(T::zero());
                }

                resample(self);

                for ch in self.in_buf.iter_mut() {
                    ch.fill(T::zero());
                }

                resample(self);
            } else {
                resample(self);
            }

            self.in_buf_len = 0;
        }
    }

    /// Resample the given input data in interleaved format.
    ///
    /// * `input` - The input data in interleaved format.
    /// * `on_processed` - Called whenever there is new resampled data. This data is in
    /// interleaved format.
    /// * `is_last_packet` - Whether or not this is the last (or only) packet of data that
    /// will be resampled. This ensures that any leftover samples in the internal resampler
    /// are flushed to the output.
    ///
    /// If you want to flush the remaining samples out of the internal resampler when there
    /// is no more input data left, set the `input` to an empty slice (`&[]`), and set
    /// `is_last_packet` to `true`.
    ///
    /// Note, when flushing the remaining data with `is_last_packet`, the resulting output
    /// may have a few extra padded zero samples on the end.
    ///
    /// The delay of the internal resampler is automatically accounted for (the starting
    /// padded zero frames are automatically truncated). Call [`NonRtResampler::reset()`]
    /// before reusing this resampler for a new sample.
    ///
    /// Note, this method is *NOT* realtime-safe.
    pub fn process_interleaved(
        &mut self,
        input: &[T],
        mut on_processed: impl FnMut(&[T]),
        is_last_packet: bool,
    ) {
        let num_channels = self.num_channels.get();

        let mut resample = |self_: &mut Self| {
            let (_, out_frames) = self_
                .resampler
                .process_into_buffer(&self_.in_buf, &mut self_.out_buf, None)
                .unwrap();

            if self_.delay_frames_left < out_frames {
                if num_channels == 1 {
                    // Mono, no need to copy to an intermediate buffer.
                    (on_processed)(&self_.out_buf[0][self_.delay_frames_left..out_frames]);
                } else {
                    crate::interleave::interleave(
                        &self_.out_buf,
                        &mut self_.intlv_buf,
                        0,
                        0,
                        out_frames,
                    );

                    (on_processed)(
                        &self_.intlv_buf
                            [self_.delay_frames_left * num_channels..out_frames * num_channels],
                    );
                }

                self_.delay_frames_left = 0;
            } else {
                self_.delay_frames_left -= out_frames;
            }
        };

        if input.is_empty() && is_last_packet {
            for b in self.in_buf.iter_mut() {
                b.fill(T::zero());
            }

            resample(self);

            return;
        }

        let total_in_frames = input.len() / num_channels;

        if self.intlv_buf.is_empty() {
            let alloc_frames = self.resampler.output_frames_max() * num_channels;

            self.intlv_buf.reserve_exact(alloc_frames);
            self.intlv_buf.resize(alloc_frames, T::zero());
        }

        let mut in_frames_copied = 0;
        while in_frames_copied < total_in_frames {
            if self.in_buf_len < self.input_frames_max {
                let copy_frames = (self.input_frames_max - self.in_buf_len)
                    .min(total_in_frames - in_frames_copied);

                crate::interleave::deinterleave(
                    input,
                    &mut self.in_buf,
                    in_frames_copied,
                    self.in_buf_len,
                    copy_frames,
                );

                self.in_buf_len += copy_frames;
                in_frames_copied += copy_frames;

                if self.in_buf_len < self.input_frames_max && !is_last_packet {
                    // Must wait for more input data before continuing.
                    return;
                }
            }

            if is_last_packet && in_frames_copied == total_in_frames {
                for ch in self.in_buf.iter_mut() {
                    ch[self.in_buf_len..].fill(T::zero());
                }

                resample(self);

                for ch in self.in_buf.iter_mut() {
                    ch.fill(T::zero());
                }

                resample(self);
            } else {
                resample(self);
            }

            self.in_buf_len = 0;
        }
    }

    pub fn resampler_type(&self) -> &ResamplerType<T> {
        &self.resampler
    }

    pub fn resampler_type_mut(&mut self) -> &mut ResamplerType<T> {
        &mut self.resampler
    }
}

impl<T: Sample> Into<ResamplerType<T>> for NonRtResampler<T> {
    fn into(self) -> ResamplerType<T> {
        self.resampler
    }
}
