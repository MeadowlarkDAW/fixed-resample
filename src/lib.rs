pub(crate) mod interleave;
mod non_realtime;
mod realtime;
mod resampler_type;

pub use non_realtime::*;
pub use realtime::*;
pub use resampler_type::*;

#[cfg(feature = "channel")]
mod channel;
#[cfg(feature = "channel")]
pub use channel::*;

pub use rubato;

/// The quality of the resampling algorithm to use.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ResampleQuality {
    /// Low quality, fast performance
    ///
    /// More specifically, this uses the [`FastFixedIn`] resampler from
    /// rubato with an interpolation type of [`PolynomialDegree::Linear`]
    /// and a chunk size of `1024`.
    Low,
    /// Great quality, medium performance
    ///
    /// This is recommended for most applications.
    ///
    /// More specifically, if the `fft-resampler` feature is enabled (which
    /// it is by default), then this uses the [`FftFixedIn`] resampler from
    /// rubato with a chunk size of `1024` and 2 sub chunks.
    ///
    /// If the `fft-resampler` feature is not enabled, then this will fall
    /// back to the `Low` quality.
    #[default]
    Normal,
}
