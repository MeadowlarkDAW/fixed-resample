use std::num::NonZeroUsize;

use rubato::{FastFixedIn, ResampleResult, Resampler as RubatoResampler, Sample};

use crate::ResampleQuality;

/// The resampling algorithm used in `fixed_resample`.
pub enum ResamplerType<T: Sample> {
    /// Ok quality, fast performance
    Fast(FastFixedIn<T>),
    #[cfg(feature = "fft-resampler")]
    /// Great quality, medium performance
    Fft(rubato::FftFixedIn<T>),
}

impl<T: Sample> ResamplerType<T> {
    pub fn from_quality(
        in_sample_rate: u32,
        out_sample_rate: u32,
        num_channels: NonZeroUsize,
        quality: ResampleQuality,
    ) -> Self {
        assert_ne!(in_sample_rate, 0);
        assert_ne!(out_sample_rate, 0);

        #[cfg(feature = "fft-resampler")]
        if let ResampleQuality::High = quality {
            return Self::Fft(
                rubato::FftFixedIn::new(
                    in_sample_rate as usize,
                    out_sample_rate as usize,
                    1024,
                    2,
                    num_channels.get(),
                )
                .unwrap(),
            );
        }

        #[cfg(not(feature = "fft-resampler"))]
        let _ = quality;

        Self::Fast(
            FastFixedIn::new(
                out_sample_rate as f64 / in_sample_rate as f64,
                1.0,
                rubato::PolynomialDegree::Linear,
                1024,
                num_channels.get(),
            )
            .unwrap(),
        )
    }

    /// Get the number of channels this Resampler is configured for.
    pub fn num_channels(&self) -> usize {
        match self {
            Self::Fast(r) => r.nbr_channels(),
            #[cfg(feature = "fft-resampler")]
            Self::Fft(r) => r.nbr_channels(),
        }
    }

    /// Reset the resampler state and clear all internal buffers.
    pub fn reset(&mut self) {
        match self {
            Self::Fast(r) => r.reset(),
            #[cfg(feature = "fft-resampler")]
            Self::Fft(r) => r.reset(),
        }
    }

    /// Get the number of frames per channel needed for the next call to
    /// [`ResamplerRefMut::process_into_buffer`].
    pub fn input_frames_next(&mut self) -> usize {
        match self {
            Self::Fast(r) => r.input_frames_next(),
            #[cfg(feature = "fft-resampler")]
            Self::Fft(r) => r.input_frames_next(),
        }
    }

    /// Get the maximum number of input frames per channel the resampler could require.
    pub fn input_frames_max(&mut self) -> usize {
        match self {
            Self::Fast(r) => r.input_frames_max(),
            #[cfg(feature = "fft-resampler")]
            Self::Fft(r) => r.input_frames_max(),
        }
    }

    /// Get the delay for the resampler, reported as a number of output frames.
    pub fn output_delay(&mut self) -> usize {
        match self {
            Self::Fast(r) => r.output_delay(),
            #[cfg(feature = "fft-resampler")]
            Self::Fft(r) => r.output_delay(),
        }
    }

    /// Get the max number of output frames per channel.
    pub fn output_frames_max(&mut self) -> usize {
        match self {
            Self::Fast(r) => r.output_frames_max(),
            #[cfg(feature = "fft-resampler")]
            Self::Fft(r) => r.output_frames_max(),
        }
    }

    /// Resample a buffer of audio to a pre-allocated output buffer.
    /// Use this in real-time applications where the unpredictable time required to allocate
    /// memory from the heap can cause glitches. If this is not a problem, you may use
    /// the [process](Resampler::process) method instead.
    ///
    /// The input and output buffers are used in a non-interleaved format.
    /// The input is a slice, where each element of the slice is itself referenceable
    /// as a slice ([AsRef<\[T\]>](AsRef)) which contains the samples for a single channel.
    /// Because `[Vec<T>]` implements [`AsRef<\[T\]>`](AsRef), the input may be [`Vec<Vec<T>>`](Vec).
    ///
    /// The output data is a slice, where each element of the slice is a `[T]` which contains
    /// the samples for a single channel. If the output channel slices do not have sufficient
    /// capacity for all output samples, the function will return an error with the expected
    /// size. You could allocate the required output buffer with
    /// [output_buffer_allocate](Resampler::output_buffer_allocate) before calling this function
    /// and reuse the same buffer for each call.
    ///
    /// The `active_channels_mask` is optional.
    /// Any channel marked as inactive by a false value will be skipped during processing
    /// and the corresponding output will be left unchanged.
    /// If `None` is given, all channels will be considered active.
    ///
    /// Before processing, it checks that the input and outputs are valid.
    /// If either has the wrong number of channels, or if the buffer for any channel is too short,
    /// a [ResampleError] is returned.
    /// Both input and output are allowed to be longer than required.
    /// The number of input samples consumed and the number output samples written
    /// per channel is returned in a tuple, `(input_frames, output_frames)`.
    pub fn process_into_buffer<Vin: AsRef<[T]>, Vout: AsMut<[T]>>(
        &mut self,
        wave_in: &[Vin],
        wave_out: &mut [Vout],
        active_channels_mask: Option<&[bool]>,
    ) -> ResampleResult<(usize, usize)> {
        match self {
            Self::Fast(r) => r.process_into_buffer(wave_in, wave_out, active_channels_mask),
            #[cfg(feature = "fft-resampler")]
            Self::Fft(r) => r.process_into_buffer(wave_in, wave_out, active_channels_mask),
        }
    }
}

impl<T: Sample> From<FastFixedIn<T>> for ResamplerType<T> {
    fn from(r: FastFixedIn<T>) -> Self {
        Self::Fast(r)
    }
}

#[cfg(feature = "fft-resampler")]
impl<T: Sample> From<rubato::FftFixedIn<T>> for ResamplerType<T> {
    fn from(r: rubato::FftFixedIn<T>) -> Self {
        Self::Fft(r)
    }
}
