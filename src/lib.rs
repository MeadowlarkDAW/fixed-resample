pub mod interleave;

#[cfg(feature = "resampler")]
mod resampler;
#[cfg(feature = "resampler")]
mod resampler_type;

#[cfg(feature = "resampler")]
pub use resampler::*;
#[cfg(feature = "resampler")]
pub use resampler_type::*;

#[cfg(feature = "channel")]
mod channel;
#[cfg(feature = "channel")]
pub use channel::*;

#[cfg(feature = "resampler")]
pub use rubato;

#[cfg(feature = "resampler")]
pub use rubato::Sample;

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
