# fixed_resample

[![Documentation](https://docs.rs/fixed_resample/badge.svg)](https://docs.rs/fixed_resample)
[![Crates.io](https://img.shields.io/crates/v/fixed_resample.svg)](https://crates.io/crates/fixed_resample)
[![License](https://img.shields.io/crates/l/fixed_resample.svg)](https://github.com/MeadowlarkDAW/fixed_resample/blob/main/LICENSE)

An easy to use crate for resampling at a fixed ratio. It supports resampling in both realtime and in non-realtime applications, and also includes a handy spsc ring buffer type that automatically resamples the input stream to match the output stream when needed.

This crate uses [Rubato](https://github.com/henquist/rubato) internally.
