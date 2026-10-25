//! Microphone capture with cpal into memory, and packing a recording as WAV
//! or FLAC for the cloud engines. Nothing is written to disk.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hound::{SampleFormat, WavSpec, WavWriter};
use std::io::Cursor;
use std::sync::{Arc, Mutex, MutexGuard};

/// Whisper works on 16 kHz mono audio
pub const TARGET_SAMPLE_RATE: u32 = 16000;

/// An input device as Basics lists it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MicDevice {
    pub name: String,
    /// The system's default input
    pub is_default: bool,
}

/// The input devices, the system's default one marked.
pub fn list_microphones() -> Vec<MicDevice> {
    let host = cpal::default_host();
    let default = host.default_input_device().and_then(|device| device.name().ok());
    let devices = match host.input_devices() {
        Ok(devices) => devices,
        Err(e) => {
            log::error!("Failed to list input devices: {}", e);
            return Vec::new();
        }
    };
    devices
        .filter_map(|device| device.name().ok())
        .map(|name| MicDevice { is_default: default.as_ref() == Some(&name), name })
        .collect()
}

/// Audio captured by one recording, already mixed to mono and resampled to 16 kHz.
pub struct CapturedAudio {
    pub samples: Vec<f32>,
    pub duration_secs: f32,
    /// Peak absolute amplitude, 0.0..=1.0
    pub peak: f32,
}

/// A running capture and the format its samples come in.
struct Capture {
    /// Dropping it ends the capture and lets the device go
    stream: cpal::Stream,
    sample_rate: u32,
    channels: u16,
}

// cpal's stream isn't `Send` on every platform. The recorder sits behind a
// mutex shared by the hotkey and command threads, and the stream is only
// ever created and dropped under it, never used from two threads at once.
unsafe impl Send for Capture {}

/// Records from one microphone at a time.
#[derive(Default)]
pub struct AudioRecorder {
    /// What the device has delivered so far, its channels interleaved
    buffer: Arc<Mutex<Vec<f32>>>,
    capture: Option<Capture>,
}

impl AudioRecorder {
    /// Opens `microphone` (a device name; "default" or empty for the system's
    /// default input) and starts collecting its samples. A device that is
    /// gone falls back to the default one.
    pub fn start(&mut self, microphone: &str) -> Result<(), String> {
        // A capture left running ends here, with whatever it collected
        self.capture = None;
        lock(&self.buffer).clear();

        let device = pick_device(microphone)?;
        let name = device.name().unwrap_or_else(|_| "<unknown>".to_string());
        // The device's own format: many refuse to record at 16 kHz directly,
        // the samples are converted once the recording ends
        let format = device
            .default_input_config()
            .map_err(|e| format!("Failed to get default input config for '{}': {}", name, e))?;
        log::info!(
            "Microphone '{}': {} Hz, {} channel(s), {:?}",
            name,
            format.sample_rate().0,
            format.channels(),
            format.sample_format()
        );

        let sink = Arc::clone(&self.buffer);
        let stream = device
            .build_input_stream(
                &format.config(),
                move |data: &[f32], _: &cpal::InputCallbackInfo| lock(&sink).extend_from_slice(data),
                |e| log::error!("Audio stream error: {}", e),
                None,
            )
            .map_err(|e| format!("Failed to open audio stream on '{}': {}", name, e))?;
        stream.play().map_err(|e| format!("Failed to start audio stream: {}", e))?;

        self.capture = Some(Capture { stream, sample_rate: format.sample_rate().0, channels: format.channels() });
        log::info!("Audio capture started");
        Ok(())
    }

    /// Stops capturing and returns what was recorded.
    pub fn stop(&mut self) -> Result<CapturedAudio, String> {
        let Some(Capture { stream, sample_rate, channels }) = self.capture.take() else {
            return Err("The microphone isn't recording".to_string());
        };
        drop(stream);
        let interleaved = std::mem::take(&mut *lock(&self.buffer));
        log::info!("Audio capture stopped, {} raw samples", interleaved.len());
        if interleaved.is_empty() {
            return Err(
                "No audio captured: the microphone delivered no data. Check the selected input device."
                    .to_string(),
            );
        }

        let mono = mix_to_mono(interleaved, channels);
        let sample_rate = sample_rate.max(1);
        let duration_secs = mono.len() as f32 / sample_rate as f32;
        let peak = mono.iter().map(|sample| sample.abs()).fold(0.0, f32::max);
        let samples = resample(&mono, sample_rate, TARGET_SAMPLE_RATE);
        log::info!(
            "Recorded {:.2} s, peak level {:.1} dBFS, {} samples at 16 kHz",
            duration_secs,
            to_dbfs(peak),
            samples.len()
        );
        Ok(CapturedAudio { samples, duration_secs, peak })
    }
}

/// The buffer shared with the audio callback. A panic elsewhere while it was
/// held leaves the samples usable, so a poisoned lock is taken over.
fn lock(buffer: &Mutex<Vec<f32>>) -> MutexGuard<'_, Vec<f32>> {
    buffer.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The device named `microphone`, or the default input for "default", an
/// empty name, or a device that isn't there any more.
fn pick_device(microphone: &str) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    let wanted = microphone.trim();
    if !wanted.is_empty() && wanted != "default" {
        let mut devices = host
            .input_devices()
            .map_err(|e| format!("Failed to list input devices: {}", e))?;
        if let Some(device) = devices.find(|device| device.name().is_ok_and(|name| name == wanted)) {
            return Ok(device);
        }
        log::warn!("Microphone '{}' not found, falling back to the default input device", wanted);
        return host.default_input_device().ok_or(format!(
            "Microphone '{}' not found and no default input device is available",
            wanted
        ));
    }
    host.default_input_device().ok_or_else(|| "No default input device found".to_string())
}

/// One sample per frame: the average of its channels.
fn mix_to_mono(interleaved: Vec<f32>, channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return interleaved;
    }
    interleaved
        .chunks(usize::from(channels))
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect()
}

/// How a recording is packed for a cloud service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    Wav,
    /// Lossless and about half the size of WAV, so it uploads faster
    Flac,
}

impl AudioFormat {
    pub fn mime(self) -> &'static str {
        match self {
            AudioFormat::Wav => "audio/wav",
            AudioFormat::Flac => "audio/flac",
        }
    }

    pub fn file_name(self) -> &'static str {
        match self {
            AudioFormat::Wav => "audio.wav",
            AudioFormat::Flac => "audio.flac",
        }
    }
}

/// A recording as a file in memory, ready to upload.
pub struct EncodedAudio {
    pub bytes: Vec<u8>,
    pub format: AudioFormat,
}

impl EncodedAudio {
    pub fn size_kb(&self) -> usize {
        self.bytes.len() / 1024
    }
}

/// Packs 16 kHz mono samples as `format`. FLAC falls back to WAV if the
/// encoder fails, so a dictation never fails for that.
pub fn encode(samples: &[f32], format: AudioFormat) -> Result<EncodedAudio, String> {
    let started = std::time::Instant::now();
    let encoded = match format {
        AudioFormat::Flac => match encode_flac(samples) {
            Ok(bytes) => EncodedAudio { bytes, format },
            Err(e) => {
                log::warn!("FLAC encoding failed, sending WAV instead: {}", e);
                EncodedAudio { bytes: encode_wav(samples)?, format: AudioFormat::Wav }
            }
        },
        AudioFormat::Wav => EncodedAudio { bytes: encode_wav(samples)?, format },
    };
    log::debug!(
        "Audio packed as {:?}: {} KB in {} ms",
        encoded.format,
        encoded.size_kb(),
        started.elapsed().as_millis()
    );
    Ok(encoded)
}

fn to_pcm16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

/// 16 kHz mono 16-bit PCM WAV.
pub fn encode_wav(samples: &[f32]) -> Result<Vec<u8>, String> {
    let spec = WavSpec { channels: 1, sample_rate: TARGET_SAMPLE_RATE, bits_per_sample: 16, sample_format: SampleFormat::Int };
    let mut buffer = Cursor::new(Vec::with_capacity(44 + samples.len() * 2));
    let mut writer = WavWriter::new(&mut buffer, spec)
        .map_err(|e| format!("Failed to start the WAV data: {}", e))?;
    for &sample in samples {
        writer
            .write_sample(to_pcm16(sample))
            .map_err(|e| format!("Failed to write WAV data: {}", e))?;
    }
    writer
        .finalize()
        .map_err(|e| format!("Failed to finish the WAV data: {}", e))?;
    Ok(buffer.into_inner())
}

/// 16 kHz mono 16-bit FLAC.
pub fn encode_flac(samples: &[f32]) -> Result<Vec<u8>, String> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;

    let pcm: Vec<i32> = samples.iter().map(|&s| to_pcm16(s) as i32).collect();
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|(_, e)| format!("Invalid FLAC settings: {:?}", e))?;
    let source = flacenc::source::MemSource::from_samples(&pcm, 1, 16, TARGET_SAMPLE_RATE as usize);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .map_err(|e| format!("FLAC encoding failed: {:?}", e))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| format!("FLAC writing failed: {:?}", e))?;
    Ok(sink.into_inner())
}

fn to_dbfs(amplitude: f32) -> f32 {
    if amplitude <= 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * amplitude.log10()
    }
}

/// Converts `input` from `from` Hz to `to` Hz, each output sample a straight
/// line between the two input samples around it. Plain, but enough for
/// speech going to a recognizer.
fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let step = f64::from(from) / f64::from(to);
    let last = input.len() - 1;
    let len = (input.len() as f64 / step) as usize;
    (0..len)
        .map(|n| {
            let position = n as f64 * step;
            let left = (position as usize).min(last);
            let right = (left + 1).min(last);
            let (a, b) = (f64::from(input[left]), f64::from(input[right]));
            (a + (b - a) * (position - left as f64)) as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_keeps_the_level_and_scales_the_length() {
        let steady = vec![0.25f32; 44100];
        let converted = resample(&steady, 44100, TARGET_SAMPLE_RATE);
        assert_eq!(converted.len(), 16000);
        assert!(converted.iter().all(|&s| (s - 0.25).abs() < 1e-6));
        assert!(resample(&[], 48000, TARGET_SAMPLE_RATE).is_empty());
    }

    #[test]
    fn resampling_draws_lines_between_samples() {
        assert_eq!(resample(&[0.0, 1.0], 1, 2), vec![0.0, 0.5, 1.0, 1.0]);
        assert_eq!(resample(&[0.0, 0.5, 1.0, 0.5], 2, 1), vec![0.0, 1.0]);
    }

    #[test]
    fn stereo_is_averaged_to_mono() {
        assert_eq!(mix_to_mono(vec![1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
        assert_eq!(mix_to_mono(vec![0.3, 0.6], 1), vec![0.3, 0.6]);
    }

    /// A second of a 440 Hz tone with a little noise, like speech it is not
    /// silent and compresses well
    fn tone() -> Vec<f32> {
        (0..TARGET_SAMPLE_RATE)
            .map(|i| {
                let t = i as f32 / TARGET_SAMPLE_RATE as f32;
                0.3 * (t * 440.0 * std::f32::consts::TAU).sin() + 0.01 * ((i * 7919 % 101) as f32 / 101.0 - 0.5)
            })
            .collect()
    }

    #[test]
    fn test_wav_in_memory() {
        let samples = tone();
        let wav = encode_wav(&samples).unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(wav.len(), 44 + samples.len() * 2);
        let reader = hound::WavReader::new(Cursor::new(wav)).unwrap();
        assert_eq!(reader.spec().sample_rate, TARGET_SAMPLE_RATE);
        assert_eq!(reader.len() as usize, samples.len());
    }

    #[test]
    fn test_flac_is_smaller_than_wav() {
        let samples = tone();
        let flac = encode(&samples, AudioFormat::Flac).unwrap();
        assert_eq!(flac.format, AudioFormat::Flac);
        assert_eq!(&flac.bytes[..4], b"fLaC");
        assert!(flac.bytes.len() < encode_wav(&samples).unwrap().len() * 3 / 4);
        assert_eq!(flac.format.mime(), "audio/flac");
    }

    #[test]
    fn test_to_dbfs() {
        assert_eq!(to_dbfs(1.0), 0.0);
        assert!(to_dbfs(0.0).is_infinite());
    }
}
