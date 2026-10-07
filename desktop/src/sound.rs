//! Audio input. Two sources: a microphone (by name, or the Windows default) and the system output via
//! WASAPI loopback ("what the other side of a call says"). A `Tap` exists only while recording.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Mic,
    System,
}

/// A live capture. Drop it to stop recording and release the device.
pub struct Tap {
    _stream: cpal::Stream,
    pub rate: u32,
    samples: Arc<Mutex<Vec<f32>>>,
    level: Arc<AtomicU32>,
    source: Source,
    device: String,
    follows_default: bool, // no device chosen in Settings (or it's gone): track Windows' default
    failed: Arc<AtomicBool>,
}

fn default_device(source: Source) -> Option<String> {
    let host = cpal::default_host();
    match source {
        Source::Mic => host.default_input_device(),
        Source::System => host.default_output_device(),
    }?
    .name()
    .ok()
}

pub fn microphones() -> Vec<String> {
    cpal::default_host()
        .input_devices()
        .map(|it| it.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

impl Tap {
    /// `mic`: device name from Settings; None or not found → Windows default microphone.
    pub fn open(source: Source, mic: Option<&str>) -> Result<Tap, String> {
        let host = cpal::default_host();
        let (device, config, follows_default) = match source {
            Source::Mic => {
                let named = mic.and_then(|want| {
                    host.input_devices().ok()?.find(|d| d.name().map(|n| n == want).unwrap_or(false))
                });
                let follows_default = named.is_none();
                let device = named
                    .or_else(|| host.default_input_device())
                    .ok_or("No microphone found. Connect one or choose it in Settings → General.")?;
                let config = device
                    .default_input_config()
                    .map_err(|e| format!("Can't use the microphone ({e}). Check Windows Settings → Privacy → Microphone."))?;
                (device, config, follows_default)
            }
            Source::System => {
                let device = host.default_output_device().ok_or("No speakers or headphones to capture the call from.")?;
                // cpal turns an input stream on an output device into WASAPI loopback capture.
                let config = device.default_output_config().map_err(|e| format!("Can't capture computer audio ({e})"))?;
                (device, config, true)
            }
        };
        let samples: Arc<Mutex<Vec<f32>>> = Arc::default();
        let level: Arc<AtomicU32> = Arc::default();
        let failed: Arc<AtomicBool> = Arc::default();
        let stream_config: cpal::StreamConfig = config.clone().into();
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => listen::<f32>(&device, &stream_config, &samples, &level, &failed),
            cpal::SampleFormat::I16 => listen::<i16>(&device, &stream_config, &samples, &level, &failed),
            cpal::SampleFormat::U16 => listen::<u16>(&device, &stream_config, &samples, &level, &failed),
            other => return Err(format!("Audio format {other:?} isn't supported")),
        }?;
        stream.play().map_err(|e| format!("Audio capture didn't start: {e}"))?;
        let device = device.name().unwrap_or_default();
        Ok(Tap { _stream: stream, rate: config.sample_rate().0, samples, level, source, device, follows_default, failed })
    }

    /// The device stopped (unplugged, Bluetooth dropped) or Windows switched the default device this tap follows:
    /// reopen it, or that side of the call goes silent.
    pub fn stale(&self) -> bool {
        self.failed.load(Ordering::Relaxed) || self.follows_default && default_device(self.source).as_deref() != Some(&self.device)
    }

    /// RMS of the latest audio packet, for the level meter.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    /// Everything captured since the last call.
    pub fn take(&self) -> Vec<f32> {
        std::mem::take(&mut *self.samples.lock().unwrap())
    }
}

fn listen<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    samples: &Arc<Mutex<Vec<f32>>>,
    level: &Arc<AtomicU32>,
    failed: &Arc<AtomicBool>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let (samples, level, failed, channels) = (samples.clone(), level.clone(), failed.clone(), config.channels as usize);
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                let mut buf = samples.lock().unwrap();
                let mut energy = 0.0f32;
                for frame in data.chunks(channels) {
                    let mono = frame.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / channels as f32;
                    energy += mono * mono;
                    buf.push(mono);
                }
                let frames = (data.len() / channels).max(1) as f32;
                level.store((energy / frames).sqrt().to_bits(), Ordering::Relaxed);
            },
            move |err| {
                eprintln!("audio stream: {err}");
                failed.store(true, Ordering::Relaxed);
            },
            None,
        )
        .map_err(|e| format!("Can't open the audio device: {e}"))
}

/// Whisper mode: quiet speech (soft voice, far mic) is raised toward a normal level so voice detection
/// doesn't throw it away. Loud audio is left alone; gain is capped so silence isn't turned into hiss.
pub fn lift_quiet(samples: &mut [f32]) {
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if peak > 0.002 && peak < 0.5 {
        let gain = (0.9 / peak).min(8.0);
        samples.iter_mut().for_each(|s| *s *= gain);
    }
}

/// Mono samples → 16-bit PCM WAV (the engine resamples to 16 kHz).
pub fn wav(samples: &[f32], rate: u32) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    for (tag, bytes) in [
        (&b"RIFF"[..], (36 + data).to_le_bytes().to_vec()),
        (b"WAVE", vec![]),
        (b"fmt ", 16u32.to_le_bytes().to_vec()),
    ] {
        out.extend_from_slice(tag);
        out.extend_from_slice(&bytes);
    }
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_layout() {
        let w = wav(&[0.0, 1.0, -1.0, 3.0], 48_000);
        assert_eq!(w.len(), 52);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(w[24..28].try_into().unwrap()), 48_000);
        assert_eq!(&w[36..40], b"data");
        assert_eq!(i16::from_le_bytes([w[50], w[51]]), 32767); // clamped
    }

    #[test]
    fn quiet_speech_is_lifted_loud_is_not() {
        let mut quiet = vec![0.05, -0.1, 0.02];
        lift_quiet(&mut quiet);
        assert!((quiet[1] + 0.8).abs() < 1e-6, "{quiet:?}"); // capped at 8x
        let mut loud = vec![0.7, -0.6];
        lift_quiet(&mut loud);
        assert_eq!(loud, [0.7, -0.6]);
    }

    /// Hardware check: `cargo test -- --ignored loopback`
    #[test]
    #[ignore]
    fn loopback_opens() {
        let tap = Tap::open(Source::System, None).expect("loopback");
        std::thread::sleep(std::time::Duration::from_millis(600));
        println!("loopback {} Hz, {} samples", tap.rate, tap.take().len());
    }
}
