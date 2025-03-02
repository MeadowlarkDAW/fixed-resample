use crate::Sample;

pub fn interleave<T: Sample, Vin: AsRef<[T]>>(
    input: &[Vin],
    output: &mut [T],
    num_out_channels: usize,
    in_start_frame: usize,
    frames: usize,
) {
    if num_out_channels == 1 {
        // Mono, no need to interleave.
        output[..frames]
            .copy_from_slice(&input[0].as_ref()[in_start_frame..in_start_frame + frames]);

        return;
    }

    if num_out_channels == 2 && input.len() >= 2 {
        // Provide an optimized loop for stereo.
        let in_ch_1 = &input[0].as_ref()[in_start_frame..in_start_frame + frames];
        let in_ch_2 = &input[1].as_ref()[in_start_frame..in_start_frame + frames];

        for ((os, &s1), &s2) in output.chunks_exact_mut(2).zip(in_ch_1).zip(in_ch_2) {
            os[0] = s1;
            os[1] = s2
        }

        return;
    }

    for (ch_i, in_ch) in input.iter().enumerate() {
        if ch_i >= num_out_channels {
            break;
        }

        for (os, &is) in output
            .chunks_exact_mut(num_out_channels)
            .zip(&in_ch.as_ref()[in_start_frame..in_start_frame + frames])
        {
            os[ch_i] = is;
        }
    }
}

pub fn deinterleave<T: Sample, Vout: AsMut<[T]>>(
    input: &[T],
    output: &mut [Vout],
    num_in_channels: usize,
    out_start_frame: usize,
    frames: usize,
) {
    if num_in_channels == 1 {
        // Mono, no need to deinterleave.
        output[0].as_mut()[out_start_frame..out_start_frame + frames].copy_from_slice(input);

        return;
    }

    if num_in_channels == 2 && output.len() >= 2 {
        // Provide an optimized loop for stereo.
        let (out_ch_1, out_ch_2) = output.split_first_mut().unwrap();

        for ((s, os1), os2) in input
            .chunks_exact(2)
            .zip(out_ch_1.as_mut()[out_start_frame..out_start_frame + frames].iter_mut())
            .zip(out_ch_2[0].as_mut()[out_start_frame..out_start_frame + frames].iter_mut())
        {
            *os1 = s[0];
            *os2 = s[1];
        }

        return;
    }

    for (ch_i, out_ch) in output.iter_mut().enumerate() {
        if ch_i >= num_in_channels {
            break;
        }

        for (s, os) in input
            .chunks_exact(num_in_channels)
            .zip(&mut out_ch.as_mut()[out_start_frame..out_start_frame + frames])
        {
            *os = s[ch_i];
        }
    }
}
