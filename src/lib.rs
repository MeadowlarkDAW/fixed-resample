mod non_realtime;
mod realtime;
mod resampler_type;

pub use non_realtime::*;
pub use realtime::*;
pub use resampler_type::*;

pub use rubato;

/// The quality of the resampling algorithm to use.
#[repr(u32)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResampleQuality {
    /// Low quality, fast performance
    ///
    /// More specifically, this uses the [`FastFixedIn`] resampler from
    /// rubato with an interpolation type of [`PolynomialDegree::Linear`]
    /// and a chunk size of `1024`.
    Low,
    /// Good quality, medium performance
    ///
    /// This is recommended for most applications.
    ///
    /// More specifically, if the `fft-resampler` feature is enabled (which
    /// it is by default), then this uses the [`FftFixedIn`] resampler from
    /// rubato with a chunk size of `1024` and 2 sub chunks.
    ///S
    /// If the `fft-resampler` feature is not enabled then this uses the
    /// [`FastFixedIn`] resampler from rubato with an interpolation type of
    /// [`PolynomialDegree::Cubic`] and a chunk size of `1024`.
    #[default]
    Normal,
    /// High quality, slow performance
    ///
    /// More specifically, this uses the [`SincFixedIn`] resampler from
    /// rubato with the following parameters:
    ///
    /// ```ignore
    /// SincInterpolationParameters {
    ///     sinc_len: 256,
    ///     f_cutoff: rubato::calculate_cutoff(256, WindowFunction::Blackman2),
    ///     interpolation: SincInterpolationType::Quadratic,
    ///     oversampling_factor: 256,
    ///     window: WindowFunction::Blackman2,
    /// }
    /// ```
    High,
    /// Very high quality, slowest performance
    ///
    /// More specifically, this uses the [`SincFixedIn`] resampler from
    /// rubato with the following parameters:
    ///
    /// ```ignore
    /// SincInterpolationParameters {
    ///     sinc_len: 512,
    ///     f_cutoff: rubato::calculate_cutoff(512, WindowFunction::BlackmanHarris2),
    ///     interpolation: SincInterpolationType::Cubic,
    ///     oversampling_factor: 512,
    ///     window: WindowFunction::BlackmanHarris2,
    /// }
    /// ```
    VeryHigh,
}
