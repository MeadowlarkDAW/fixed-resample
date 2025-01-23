use rubato::Sample;

use crate::{ResampleQuality, ResamplerType};

/// An easy to use resampler for use in non-realtime applications.
pub struct NonRtResampler<T: Sample> {
    resampler: ResamplerType<T>,
    in_buf: Vec<Vec<T>>,
    out_buf: Vec<Vec<T>>,
    in_buf_len: usize,
    num_channels: usize,
    input_frames_max: usize,
}

impl<T: Sample> NonRtResampler<T> {
    /// Create a new non-realtime resampler.
    ///
    /// * `in_sample_rate` - The sample rate of the input stream.
    /// * `out_sample_rate` - The sample rate of the output stream.
    /// * `num_channels` - The number of channels.
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

        Self {
            resampler,
            in_buf: (0..num_channels)
                .map(|_| vec![T::zero(); input_frames_max])
                .collect(),
            out_buf: (0..num_channels)
                .map(|_| vec![T::zero(); output_frames_max])
                .collect(),
            in_buf_len: 0,
            num_channels,
            input_frames_max,
        }
    }

    /// Get the number of channels this resampler is configured for.
    pub fn num_channels(&self) -> usize {
        self.num_channels
    }

    /// Reset the resampler state and clear all internal buffers.
    pub fn reset(&mut self) {
        self.resampler.reset();

        self.in_buf_len = 0;
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
    /// Note, this method is *NOT* realtime-safe.
    pub fn process<Vin: AsRef<[T]>>(
        &mut self,
        input: &[Vin],
        mut on_processed: impl FnMut(&[Vec<T>], usize),
        is_last_packet: bool,
    ) {
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
                let mut is_first = true;

                loop {
                    let (_, out_frames) = self
                        .resampler
                        .process_partial_into_buffer(
                            if is_first { Some(&self.in_buf) } else { None },
                            &mut self.out_buf,
                            None,
                        )
                        .unwrap();

                    is_first = false;

                    (on_processed)(self.out_buf.as_slice(), out_frames);

                    if out_frames < self.out_buf[0].len() {
                        break;
                    }
                }
            } else {
                let (_, out_frames) = self
                    .resampler
                    .process_into_buffer(&self.in_buf, &mut self.out_buf, None)
                    .unwrap();

                (on_processed)(self.out_buf.as_slice(), out_frames);
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
