use rubato::Sample;

pub fn interleave<T: Sample>(
    input: &[Vec<T>],
    output: &mut [T],
    in_start_frame: usize,
    out_start_frame: usize,
    frames: usize,
) {
    let num_channels = input.len();

    let out_slice =
        &mut output[out_start_frame * num_channels..(out_start_frame + frames) * num_channels];

    match num_channels {
        1 => {
            // Mono, no need to interleave.
            out_slice.copy_from_slice(&input[0][in_start_frame..in_start_frame + frames]);
        }
        2 => {
            // Provide an optimized loop for stereo.
            let in_ch_1 = &input[0][in_start_frame..in_start_frame + frames];
            let in_ch_2 = &input[1][in_start_frame..in_start_frame + frames];

            for ((os, &s1), &s2) in out_slice.chunks_exact_mut(2).zip(in_ch_1).zip(in_ch_2) {
                os[0] = s1;
                os[1] = s2
            }
        }
        _ => {
            assert_eq!(input.len(), num_channels);

            for (ch_i, in_ch) in input.iter().enumerate() {
                for (os, &is) in out_slice
                    .chunks_exact_mut(num_channels)
                    .zip(&in_ch[in_start_frame..in_start_frame + frames])
                {
                    os[ch_i] = is;
                }
            }
        }
    }
}

pub fn deinterleave<T: Sample>(
    input: &[T],
    output: &mut [Vec<T>],
    in_start_frame: usize,
    out_start_frame: usize,
    frames: usize,
) {
    let num_channels = output.len();

    let in_slice = &input[in_start_frame * num_channels..(in_start_frame + frames) * num_channels];

    match num_channels {
        1 => {
            // Mono, no need to deinterleave.
            output[0][out_start_frame..out_start_frame + frames].copy_from_slice(in_slice);
        }
        2 => {
            // Provide an optimized loop for stereo.
            let (out_ch_1, out_ch_2) = output.split_first_mut().unwrap();

            for ((s, os1), os2) in in_slice
                .chunks_exact(2)
                .zip(out_ch_1[out_start_frame..out_start_frame + frames].iter_mut())
                .zip(out_ch_2[0][out_start_frame..out_start_frame + frames].iter_mut())
            {
                *os1 = s[0];
                *os2 = s[1];
            }
        }
        _ => {
            for (ch_i, out_ch) in output.iter_mut().enumerate() {
                for (s, os) in in_slice
                    .chunks_exact(num_channels)
                    .zip(&mut out_ch[out_start_frame..out_start_frame + frames])
                {
                    *os = s[ch_i];
                }
            }
        }
    }
}
