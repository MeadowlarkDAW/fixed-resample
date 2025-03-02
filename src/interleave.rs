use std::{num::NonZeroUsize, ops::Range};

use crate::Sample;

/// Interleave the given channels into the output buffer.
///
/// * `input` - The input channels.
/// * `output` - The interleaved output buffer to write to.
/// * `num_out_channels` - The number of interleaved channels in `output`.
/// * `in_range` - The range in each channel in `input` to read from.
pub fn interleave<T: Sample, Vin: AsRef<[T]>>(
    input: &[Vin],
    output: &mut [T],
    num_out_channels: NonZeroUsize,
    in_range: Range<usize>,
) {
    let in_frames = in_range.end - in_range.start;
    let out_frames = output.len() / num_out_channels.get();

    let frames = in_frames.min(out_frames);

    let in_range = in_range.start..in_range.start + frames;

    if num_out_channels.get() == 1 {
        // Mono, no need to interleave.
        output[..frames].copy_from_slice(&input[0].as_ref()[in_range.clone()]);

        return;
    }

    if num_out_channels.get() == 2 && input.len() >= 2 {
        // Provide an optimized loop for stereo.
        let in_ch_1 = &input[0].as_ref()[in_range.clone()];
        let in_ch_2 = &input[1].as_ref()[in_range.clone()];

        for ((os, &s1), &s2) in output.chunks_exact_mut(2).zip(in_ch_1).zip(in_ch_2) {
            os[0] = s1;
            os[1] = s2
        }

        return;
    }

    for (ch_i, in_ch) in input.iter().enumerate() {
        if ch_i >= num_out_channels.get() {
            break;
        }

        for (os, &is) in output
            .chunks_exact_mut(num_out_channels.get())
            .zip(&in_ch.as_ref()[in_range.clone()])
        {
            os[ch_i] = is;
        }
    }
}

/// Deinterleave the given data into the output buffers.
///
/// * `input` - The interleaved input data.
/// * `output` - The de-interleaved output buffer to write to.
/// * `num_in_channels` - The number of interleaved channels in `input`.
/// * `out_range` - The range in each channel in `output` to write to.
pub fn deinterleave<T: Sample, Vout: AsMut<[T]>>(
    input: &[T],
    output: &mut [Vout],
    num_in_channels: NonZeroUsize,
    out_range: Range<usize>,
) {
    let out_frames = out_range.end - out_range.start;
    let in_frames = input.len() / num_in_channels.get();

    let frames = out_frames.min(in_frames);

    let out_range = out_range.start..out_range.start + frames;

    if num_in_channels.get() == 1 {
        // Mono, no need to deinterleave.
        output[0].as_mut()[out_range.clone()].copy_from_slice(&input[..frames]);

        return;
    }

    if num_in_channels.get() == 2 && output.len() >= 2 {
        // Provide an optimized loop for stereo.
        let (out_ch_1, out_ch_2) = output.split_first_mut().unwrap();

        for ((s, os1), os2) in input
            .chunks_exact(2)
            .zip(out_ch_1.as_mut()[out_range.clone()].iter_mut())
            .zip(out_ch_2[0].as_mut()[out_range.clone()].iter_mut())
        {
            *os1 = s[0];
            *os2 = s[1];
        }

        return;
    }

    for (ch_i, out_ch) in output.iter_mut().enumerate() {
        if ch_i >= num_in_channels.get() {
            break;
        }

        for (s, os) in input
            .chunks_exact(num_in_channels.get())
            .zip(&mut out_ch.as_mut()[out_range.clone()])
        {
            *os = s[ch_i];
        }
    }
}
