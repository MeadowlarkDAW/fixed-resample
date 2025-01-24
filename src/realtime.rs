use std::num::NonZeroUsize;

use rubato::Sample;

use crate::{ResampleQuality, ResamplerType};

/// An easy to use resampler that can be used in realtime applications.
pub struct RtResampler<T: Sample> {
    resampler: ResamplerType<T>,
    in_buf: Vec<Vec<T>>,
    out_buf: Vec<Vec<T>>,
    intlv_buf: Vec<T>,
    out_buf_len: usize,
    remaining_frames_in_out_buf: usize,
    num_channels: NonZeroUsize,
    max_out_frames: usize,
    input_frames_max: usize,
}

impl<T: Sample> RtResampler<T> {
    /// Create a new realtime resampler.
    ///
    /// * `in_sample_rate` - The sample rate of the input stream.
    /// * `out_sample_rate` - The sample rate of the output stream.
    /// * `num_channels` - The number of channels.
    /// * `max_out_frames` - The maximum number of output frames that can appear in a
    /// single call to [`RtResampler::process`].
    /// * `interleaved` - If you are planning to use [`RtResampler::process_interleaved`],
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
    /// * `max_out_frames == 0`
    pub fn new(
        in_sample_rate: u32,
        out_sample_rate: u32,
        num_channels: usize,
        max_out_frames: usize,
        interleaved: bool,
        quality: ResampleQuality,
    ) -> Self {
        let resampler =
            ResamplerType::from_quality(in_sample_rate, out_sample_rate, num_channels, quality);
        Self::from_custom(resampler, max_out_frames, interleaved)
    }

    /// Create a new realtime resampler using the given rubato resampler.
    ///
    /// * `resampler` - The rubato resampler.
    /// * `max_out_frames` - The maximum number of output frames that can appear in a
    /// single call to [`RtResampler::process`].
    /// * `interleaved` - If you are planning to use [`RtResampler::process_interleaved`],
    /// set this to `true`. Otherwise you can set this to `false` to save a bit of
    /// memory.
    ///
    /// # Panics
    /// Panics if `max_out_frames == 0`.
    pub fn from_custom(
        resampler: impl Into<ResamplerType<T>>,
        max_out_frames: usize,
        interleaved: bool,
    ) -> Self {
        assert_ne!(max_out_frames, 0);

        let mut resampler: ResamplerType<T> = resampler.into();

        let num_channels = resampler.num_channels();
        assert_ne!(num_channels, 0);

        let input_frames_max = resampler.input_frames_max();
        let output_frames_max = resampler.output_frames_max();

        let intlv_buf = if num_channels == 1 || !interleaved {
            Vec::new()
        } else {
            let mut v = Vec::new();
            v.reserve_exact(input_frames_max * num_channels);
            v.resize(input_frames_max * num_channels, T::zero());
            v
        };

        Self {
            resampler,
            in_buf: (0..num_channels)
                .map(|_| {
                    let mut v = Vec::new();
                    v.reserve_exact(input_frames_max);
                    v.resize(input_frames_max, T::zero());
                    v
                })
                .collect(),
            out_buf: (0..num_channels)
                .map(|_| {
                    let mut v = Vec::new();
                    v.reserve_exact(output_frames_max);
                    v.resize(output_frames_max, T::zero());
                    v
                })
                .collect(),
            num_channels: NonZeroUsize::new(num_channels).unwrap(),
            intlv_buf,
            max_out_frames,
            input_frames_max,
            out_buf_len: 0,
            remaining_frames_in_out_buf: 0,
        }
    }

    /// Get the number of channels this resampler is configured for.
    pub fn num_channels(&self) -> NonZeroUsize {
        self.num_channels
    }

    /// Reset the resampler state and clear all internal buffers.
    pub fn reset(&mut self) {
        self.resampler.reset();
        self.out_buf_len = 0;
        self.remaining_frames_in_out_buf = 0;
    }

    /// The maximum number of output frames that can appear in a single call to
    /// [`RtResampler::process`].
    pub fn max_out_frames(&self) -> usize {
        self.max_out_frames
    }

    /// The maximum number of frames that can be requested in [`RtResampler::process`].
    pub fn max_request_frames(&self) -> usize {
        self.input_frames_max
    }

    /// Resample the input stream and process into a block of data for the output stream.
    ///
    /// This method is realtime-safe.
    ///
    /// * `on_frames_requested` - This gets called whenever the resampler needs more
    /// data from the input stream. The given buffer must be fully filled with new samples.
    /// If there is not enough data to fill the buffer (i.e. an underflow occured), then fill
    /// the rest of the frames with zeros. Do *NOT* resize the `Vec`s.
    /// * `output` - The output buffers to write the resampled data to.
    /// * `out_frames` - The number of frames to write to `output`. If the given value is
    /// greater than [`RtResampler::max_out_frames`], then this will panic.
    ///
    /// # Panics
    ///
    /// * Panics if the number of output channels does not equal the number of channels
    /// in this resampler.
    /// * Panics if the number of frames in the output buffers is greater than
    /// [`RtResampler::max_out_frames`].
    pub fn process<Vout: AsMut<[T]>>(
        &mut self,
        mut on_frames_requested: impl FnMut(&mut [Vec<T>]),
        output: &mut [Vout],
        out_frames: usize,
    ) {
        assert_eq!(output.len(), self.num_channels.get());
        assert!(out_frames <= self.max_out_frames);

        let mut frames_filled = if self.remaining_frames_in_out_buf > 0 {
            let start_frame = self.out_buf_len - self.remaining_frames_in_out_buf;
            let copy_frames = self.remaining_frames_in_out_buf.min(out_frames);

            for (out_ch, in_ch) in output.iter_mut().zip(self.out_buf.iter()) {
                let out_ch = out_ch.as_mut();
                out_ch[..copy_frames]
                    .copy_from_slice(&in_ch[start_frame..start_frame + copy_frames]);
            }

            self.remaining_frames_in_out_buf -= copy_frames;

            copy_frames
        } else {
            0
        };

        while frames_filled < out_frames {
            (on_frames_requested)(&mut self.in_buf);

            debug_assert!(self.in_buf[0].len() == self.input_frames_max);

            let (_, out_frames_processed) = self
                .resampler
                .process_into_buffer(&self.in_buf, &mut self.out_buf, None)
                .unwrap();

            self.out_buf_len = out_frames_processed;
            let copy_frames = out_frames_processed.min(out_frames - frames_filled);

            for (out_ch, in_ch) in output.iter_mut().zip(self.out_buf.iter()) {
                let out_ch = out_ch.as_mut();
                out_ch[frames_filled..frames_filled + copy_frames]
                    .copy_from_slice(&in_ch[..copy_frames]);
            }

            self.remaining_frames_in_out_buf = self.out_buf_len - copy_frames;

            frames_filled += copy_frames;
        }
    }

    /// Resample the input stream and process into an interleaved block of data for the
    /// output stream.
    ///
    /// * `on_frames_requested` - This gets called whenever the resampler needs more
    /// data from the input stream. The given buffer is in interleaved format, and it
    /// must be completely filled with new data. If there is not enough data to fill
    /// the buffer (i.e. an underflow occured), then fill the rest of the frames with
    /// zeros.
    /// * `output` - The interleaved output buffer to write the resampled data to.
    ///
    /// # Panics
    ///
    /// * Panics if the number of output channels does not equal the number of channels
    /// in this resampler.
    /// * Panics if the number of frames in the output buffers is greater than
    /// [`RtResampler::max_out_frames`].
    /// * Also panics if the `interleaved` argument was `false` when this struct was
    /// created.
    pub fn process_interleaved(
        &mut self,
        mut on_frames_requested: impl FnMut(&mut [T]),
        output: &mut [T],
    ) {
        let num_channels = self.num_channels.get();

        let out_frames = output.len() / num_channels;

        assert!(out_frames <= self.max_out_frames);

        if num_channels > 1 {
            assert!(!self.intlv_buf.is_empty());
        }

        let mut frames_filled = if self.remaining_frames_in_out_buf > 0 {
            let start_frame = self.out_buf_len - self.remaining_frames_in_out_buf;
            let copy_frames = self.remaining_frames_in_out_buf.min(out_frames);

            crate::interleave::interleave(&self.out_buf, output, start_frame, 0, copy_frames);

            self.remaining_frames_in_out_buf -= copy_frames;

            copy_frames
        } else {
            0
        };

        while frames_filled < out_frames {
            if num_channels == 1 {
                // Mono, no need for the temporary interleaved buffer.
                (on_frames_requested)(&mut self.in_buf[0]);
            } else {
                (on_frames_requested)(&mut self.intlv_buf);

                crate::interleave::deinterleave(
                    &self.intlv_buf,
                    &mut self.in_buf,
                    0,
                    0,
                    self.input_frames_max,
                );
            }

            let (_, out_frames_processed) = self
                .resampler
                .process_into_buffer(&self.in_buf, &mut self.out_buf, None)
                .unwrap();

            self.out_buf_len = out_frames_processed;
            let copy_frames = out_frames_processed.min(out_frames - frames_filled);

            crate::interleave::interleave(&self.out_buf, output, 0, frames_filled, copy_frames);

            self.remaining_frames_in_out_buf = self.out_buf_len - copy_frames;

            frames_filled += copy_frames;
        }
    }

    pub fn resampler_type(&self) -> &ResamplerType<T> {
        &self.resampler
    }

    pub fn resampler_type_mut(&mut self) -> &mut ResamplerType<T> {
        &mut self.resampler
    }
}

impl<T: Sample> Into<ResamplerType<T>> for RtResampler<T> {
    fn into(self) -> ResamplerType<T> {
        self.resampler
    }
}
