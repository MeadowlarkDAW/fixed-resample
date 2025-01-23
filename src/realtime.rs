use rubato::Sample;

use crate::{ResampleQuality, ResamplerType};

/// An easy to use resampler that can be used in realtime applications.
pub struct RtResampler<T: Sample> {
    resampler: ResamplerType<T>,
    in_buf: Vec<Vec<T>>,
    out_buf: Vec<Vec<T>>,
    out_buf_len: usize,
    remaining_frames_in_out_buf: usize,
    num_channels: usize,
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
        quality: ResampleQuality,
    ) -> Self {
        let resampler =
            ResamplerType::from_quality(in_sample_rate, out_sample_rate, num_channels, quality);
        Self::from_custom(resampler, max_out_frames)
    }

    /// Create a new realtime resampler using the given rubato resampler.
    ///
    /// * `resampler` - The rubato resampler.
    /// * `max_out_frames` - The maximum number of output frames that can appear in a
    /// single call to [`RtResampler::process`].
    ///
    /// # Panics
    /// Panics if `max_out_frames == 0`.
    pub fn from_custom(resampler: impl Into<ResamplerType<T>>, max_out_frames: usize) -> Self {
        assert_ne!(max_out_frames, 0);

        let mut resampler: ResamplerType<T> = resampler.into();

        let num_channels = resampler.num_channels();
        assert!(num_channels > 0);

        let input_frames_max = resampler.input_frames_max();
        let output_frames_max = resampler.output_frames_max();

        Self {
            resampler,
            in_buf: (0..num_channels)
                .map(|_| Vec::with_capacity(input_frames_max))
                .collect(),
            out_buf: (0..num_channels)
                .map(|_| vec![T::zero(); output_frames_max])
                .collect(),
            num_channels,
            max_out_frames,
            input_frames_max,
            out_buf_len: 0,
            remaining_frames_in_out_buf: 0,
        }
    }

    /// Get the number of channels this resampler is configured for.
    pub fn num_channels(&self) -> usize {
        self.num_channels
    }

    /// Reset the resampler state and clear all internal buffers.
    pub fn reset(&mut self) {
        self.resampler.reset();

        for b in self.in_buf.iter_mut() {
            b.clear();
        }

        self.out_buf_len = 0;
        self.remaining_frames_in_out_buf = 0;
    }

    /// The maximum number of output frames that can appear in a single call to
    /// [`RtResampler::process`].
    pub fn max_out_frames(&self) -> usize {
        self.max_out_frames
    }

    /// Resample the input stream and process into a block of data for the output stream.
    ///
    /// * `on_frames_requested` - This gets called whenever the resampler needs more
    /// data from the input stream. The first argument is the number of frames it requests, and
    /// the second argument is the buffers in which to append the new data to. (Note, you must
    /// *append*, not overwrite). If there are not enough frames in the input stream (i.e. an
    /// input underflow occured), then extend by how many frames you can. Do *NOT* append more
    /// frames than requested.
    /// * `output` - The output buffers to write the resampled data to.
    /// * `out_frames` - The number of frames to write to `output`. If the given value is
    /// greater than [`RtResampler::max_out_frames`], then this will panic.
    pub fn process<Vout: AsMut<[T]>>(
        &mut self,
        mut on_frames_requested: impl FnMut(usize, &mut [Vec<T>]),
        output: &mut [Vout],
        out_frames: usize,
    ) {
        assert!(out_frames <= self.max_out_frames);

        let mut frames_filled = if self.remaining_frames_in_out_buf > 0 {
            let start_frame = self.out_buf_len - self.remaining_frames_in_out_buf;
            let copy_frames = self.remaining_frames_in_out_buf.min(out_frames);

            for (out_ch, out_buf_ch) in output.iter_mut().zip(self.out_buf.iter()) {
                let out_ch = out_ch.as_mut();
                out_ch[..copy_frames]
                    .copy_from_slice(&out_buf_ch[start_frame..start_frame + copy_frames]);
            }

            self.remaining_frames_in_out_buf -= copy_frames;

            copy_frames
        } else {
            0
        };

        while frames_filled < out_frames {
            if self.in_buf[0].len() < self.input_frames_max {
                (on_frames_requested)(
                    self.input_frames_max - self.in_buf[0].len(),
                    &mut self.in_buf,
                );

                debug_assert!(self.in_buf[0].len() <= self.input_frames_max);

                if self.in_buf[0].len() < self.input_frames_max {
                    // Underflow occured. Fill the remaining dropped samples with zeros.
                    for ch in self.in_buf.iter_mut() {
                        ch.resize(self.input_frames_max, T::zero());
                    }
                }
            }

            let (_, out_frames_processed) = self
                .resampler
                .process_into_buffer(&self.in_buf, &mut self.out_buf, None)
                .unwrap();

            self.out_buf_len = out_frames_processed;
            let copy_frames = out_frames_processed.min(out_frames - frames_filled);

            for (out_ch, out_buf_ch) in output.iter_mut().zip(self.out_buf.iter()) {
                let out_ch = out_ch.as_mut();
                out_ch[frames_filled..frames_filled + copy_frames]
                    .copy_from_slice(&out_buf_ch[..copy_frames]);
            }

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
