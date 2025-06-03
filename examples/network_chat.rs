use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use fixed_resample::{PushStatus, ReadStatus, ResamplingChannelConfig};

const NETWORK_SAMPLE_RATE: usize = 44100;
const PACKET_FRAMES: usize = 2048;
const RUN_DURATION: Duration = Duration::from_secs(10);

pub fn main() {
    // This example simulates a network connection using a standard mpsc channel.
    let (network_sender, network_receiver) = std::sync::mpsc::channel();
    let network_sender_2 = network_sender.clone();

    let t1 = std::thread::spawn(move || run_microphone_client(network_sender, 1));
    let t2 = std::thread::spawn(move || run_microphone_client(network_sender_2, 2));
    let t3 = std::thread::spawn(move || run_playback_client(network_receiver, 3, &[1, 2]));

    t1.join().unwrap();
    t2.join().unwrap();
    t3.join().unwrap();
}

// A simulated network packet of samples.
//
// Note, in a real application, you would probably want to encode the sample data
// into a compressed format.
struct NetworkPacket {
    client_id: u32,
    samples: [f32; PACKET_FRAMES],
}

// Client 1 and 2 send microphone data to the network.
fn run_microphone_client(network_sender: std::sync::mpsc::Sender<NetworkPacket>, client_id: u32) {
    let host = cpal::default_host();

    let input_device = host.default_input_device().unwrap();
    let input_config = input_device.default_input_config().unwrap();

    let input_channels = input_config.channels() as usize;

    let (mut prod, mut cons) = fixed_resample::resampling_channel::<f32, 1>(
        NonZeroUsize::new(1).unwrap(),      // num_channels
        input_config.sample_rate().0,       // in_sample_rate
        NETWORK_SAMPLE_RATE as u32,         // out_sample_rate
        ResamplingChannelConfig::default(), // config
    );

    // Because the consumer end is used in a non-realtime network polling thread,
    // notify the producer end that it can start pushing samples without delay.
    cons.set_output_stream_ready(true);

    // A temporary input buffer.
    let mut tmp_buf: Vec<f32> = vec![0.0; 8192];

    let input_stream = input_device
        .build_input_stream(
            &input_config.config(),
            move |data: &[f32], _info: &cpal::InputCallbackInfo| {
                let frames = data.len() / input_channels;

                // We only care about mono microphone input, so only copy the first channel to
                // a temporary buffer.
                deinterleave_first_channel(data, input_channels, &mut tmp_buf[..frames]);

                let status = prod.push_interleaved(&tmp_buf[..frames]);

                match status {
                    // All samples were successfully pushed to the channel.
                    PushStatus::Ok => {}
                    // The output stream is not yet ready to read samples from the channel. No
                    // samples have been pushed to the channel.
                    PushStatus::OutputNotReady => {}
                    // An overflow occured due to the producer running faster than the consumer.
                    PushStatus::OverflowOccurred {
                        num_frames_pushed: _,
                    } => {
                        eprintln!(
                            "client {} consumer behind: try increasing channel capacity",
                            client_id
                        );
                    }
                    // An underflow occured due to the consumer running faster than the producer.
                    //
                    // All of the samples were successfully pushed to the channel, however extra
                    // zero samples were also pushed to the channel to correct for the jitter.
                    PushStatus::UnderflowCorrected {
                        num_zero_frames_pushed: _,
                    } => {
                        eprintln!(
                            "client {} producer fell behind: try increasing channel latency",
                            client_id
                        );
                    }
                }
            },
            move |e| eprintln!("Input stream error on client {}: {}", client_id, e),
            None,
        )
        .unwrap();
    input_stream.play().unwrap();

    let sleep_duration =
        Duration::from_secs_f64(0.25 * PACKET_FRAMES as f64 / NETWORK_SAMPLE_RATE as f64);

    let start = Instant::now();
    while start.elapsed() < RUN_DURATION {
        // Detect when a new packet of data should be pushed.
        //
        // Alternatively you could do:
        // while cons.occupied_seconds() < cons.latency_seconds() {
        while cons.available_frames() >= PACKET_FRAMES {
            let mut packet = NetworkPacket {
                client_id,
                samples: [0.0; PACKET_FRAMES],
            };
            cons.read_interleaved(packet.samples.as_mut_slice());

            network_sender.send(packet).unwrap();
        }

        std::thread::sleep(sleep_duration);
    }
}

// Client 3 receives data from the network and plays it back.
fn run_playback_client(
    network_receiver: std::sync::mpsc::Receiver<NetworkPacket>,
    client_id: u32,
    input_client_ids: &[u32],
) {
    struct InputClientChannelProd {
        client_id: u32,
        prod: fixed_resample::ResamplingProd<f32, 1>,
    }

    struct InputClientChannelCons {
        client_id: u32,
        cons: fixed_resample::ResamplingCons<f32>,
    }

    let host = cpal::default_host();

    let output_device = host.default_output_device().unwrap();
    let output_config = output_device.default_output_config().unwrap();

    let output_channels = output_config.channels() as usize;

    let mut producers: Vec<InputClientChannelProd> = Vec::new();
    let mut consumers: Vec<InputClientChannelCons> = Vec::new();
    for id in input_client_ids {
        let (mut prod, cons) = fixed_resample::resampling_channel::<f32, 1>(
            NonZeroUsize::new(1).unwrap(), // num_channels
            NETWORK_SAMPLE_RATE as u32,    // in_sample_rate
            output_config.sample_rate().0, // out_sample_rate
            ResamplingChannelConfig {
                // Because this stream is using data sent to it from over the network,
                // increase the latency and the capacity to account for delays.
                latency_seconds: 0.5,  // 500 ms
                capacity_seconds: 1.0, // 1 second
                ..Default::default()
            },
        );

        // Because the producer end is used in a non-realtime network polling thread,
        // notify the consumer end that it can start reading samples without delay.
        prod.set_input_stream_ready(true);

        producers.push(InputClientChannelProd {
            client_id: *id,
            prod,
        });
        consumers.push(InputClientChannelCons {
            client_id: *id,
            cons,
        });
    }

    // A temporary buffer for copying over samples.
    let mut tmp_input: Vec<f32> = vec![0.0; 8192];
    let mut tmp_output: Vec<f32> = vec![0.0; 8192];

    let output_stream = output_device
        .build_output_stream(
            &output_config.config(),
            move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                let frames = data.len() / output_channels;

                // Initialize the output buffer with zeros.
                tmp_output[..frames].fill(0.0);

                for cons in consumers.iter_mut() {
                    let status = cons.cons.read_interleaved(&mut tmp_input[..frames]);

                    match status {
                        // The output buffer was fully filled with samples from the channel.
                        ReadStatus::Ok => {}
                        // The producer is not yet ready to push samples to the channel.
                        // The output buffer was filled with zeros.
                        ReadStatus::InputNotReady => {}
                        // An underflow occured due to the consumer running faster than the producer.
                        //
                        // Note, when compiled in debug mode without optimizations, the resampler
                        // is quite slow, leading to frequent underflows in this example.
                        ReadStatus::UnderflowOccurred { num_frames_read } => {
                            println!(
                                "Underflow occured in client {} from client {}! Number of output frames dropped: {}",
                                client_id,
                                cons.client_id,
                                frames - num_frames_read
                            );
                        }
                        // An overflow occured due to the producer running faster than the consumer.
                        //
                        // All of the samples in the output buffer were successfully filled with samples,
                        // however a number of frames have also been discarded to correct for the jitter.
                        ReadStatus::OverflowCorrected {
                            num_frames_discarded,
                        } => {
                            println!(
                                "Overflow occured in client {} from client {}! Number of frames discarded from channel: {}",
                                client_id,
                                cons.client_id,
                                num_frames_discarded
                            );
                        }
                    }

                    // Mix the samples into the resulting output.
                    let volume = 1.0;
                    for (out_s, &in_s) in tmp_output[..frames].iter_mut().zip(tmp_input[..frames].iter()) {
                        *out_s += in_s * volume;
                    }
                }

                // Copy the resulting output to each output channel.
                interleave_mono_input(&tmp_output[..frames], data, output_channels);
            },
            move |e| eprintln!("Output stream error on client {}: {}", client_id, e),
            None,
        )
        .unwrap();
    output_stream.play().unwrap();

    let sleep_duration =
        Duration::from_secs_f64(0.25 * PACKET_FRAMES as f64 / NETWORK_SAMPLE_RATE as f64);

    let start = Instant::now();
    while start.elapsed() < RUN_DURATION {
        while let Ok(packet) = network_receiver.try_recv() {
            producers
                .iter_mut()
                .find(|p| p.client_id == packet.client_id)
                .unwrap()
                .prod
                .push_interleaved(packet.samples.as_slice());
        }

        std::thread::sleep(sleep_duration);
    }
}

fn deinterleave_first_channel(input: &[f32], input_channels: usize, mono_output: &mut [f32]) {
    let frames = input.len() / input_channels;

    match input_channels {
        1 => {
            // Mono input, just copy the samples directly.
            mono_output[..frames].copy_from_slice(&input[..frames]);
        }
        num_ch => {
            for (out_s, in_chunk) in mono_output[..frames]
                .iter_mut()
                .zip(input.chunks_exact(num_ch))
            {
                *out_s = in_chunk[0];
            }
        }
    }
}

fn interleave_mono_input(mono_input: &[f32], output: &mut [f32], output_channels: usize) {
    let frames = mono_input.len();

    match output_channels {
        1 => {
            // Mono output, just copy the samples directly.
            output[..frames].copy_from_slice(&mono_input[..frames]);
        }
        2 => {
            // Since stereo is the most common case, provide an optimized interleaving method.
            //
            // While this is probably overkill for a simple example, I just want to highlight
            // how to do this since it is much faster than the generic loop below this one.
            for (out_chunk, &in_s) in output.chunks_exact_mut(2).zip(mono_input[..frames].iter()) {
                out_chunk[0] = in_s;
                out_chunk[1] = in_s;
            }
        }
        num_ch => {
            for (out_chunk, &in_s) in output
                .chunks_exact_mut(num_ch)
                .zip(mono_input[..frames].iter())
            {
                for out_s in out_chunk.iter_mut() {
                    *out_s = in_s;
                }
            }
        }
    }
}
