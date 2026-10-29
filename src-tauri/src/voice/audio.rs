//! The microphone and the speaker for push-to-talk, on cpal.
//!
//! OpenAI's realtime transcriber and its speech endpoint both speak 24 kHz mono 16-bit PCM, while
//! devices run at whatever rate and channel count they like (usually 48 kHz stereo), so both ways
//! go through `Resampler`. Each cpal stream lives on a thread of its own and is dropped there:
//! streams are not `Send` on every platform.

use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    FromSample, Sample, SampleFormat, SizedSample, StreamConfig,
};
use tokio::sync::{mpsc::UnboundedSender, oneshot};

use crate::config::{debug_log, VOICE_END_SILENCE_MS, VOICE_MAX_SECONDS, VOICE_NO_SPEECH_MS};

/// The one rate the transcriber accepts and the speech endpoint returns.
pub const VOICE_RATE: u32 = 24_000;

/// Linear-interpolation resampler for a continuous mono stream fed in chunks. Speech has little
/// above 8 kHz, so this is all the quality the transcriber (and a voice) needs.
pub struct Resampler {
    /// Input samples per output sample.
    step: f64,
    /// Where the next output sample falls, as an index into the next chunk. Negative values fall
    /// between the previous chunk's last sample and this chunk's first.
    position: f64,
    previous: f32,
}

impl Resampler {
    pub fn new(input_rate: u32, output_rate: u32) -> Self {
        Self { step: input_rate as f64 / output_rate as f64, position: 0.0, previous: 0.0 }
    }

    pub fn process(&mut self, input: &[f32], output: &mut Vec<f32>) {
        let Some(&last) = input.last() else { return };
        let len = input.len() as f64;
        let at = |i: isize| if i < 0 { self.previous } else { input[i as usize] };
        let mut t = self.position;
        while t < len - 1.0 {
            let i = t.floor();
            let frac = (t - i) as f32;
            let (a, b) = (at(i as isize), at(i as isize + 1));
            output.push(a + (b - a) * frac);
            t += self.step;
        }
        self.position = t - len;
        self.previous = last;
    }
}

/// Automatic gain for the microphone. Browsers apply this to getUserMedia audio (the composer's
/// mic asks for it); raw device capture gets none, and on a typical laptop mic speech arrived at
/// about -38 dBFS, far below the -20 dBFS a transcriber hears best. The gain follows the level of
/// speech only, so pauses never pump the room noise up, and a soft limiter keeps it from clipping.
pub struct AutoGain {
    /// Smoothed RMS of recent speech blocks.
    speech_rms: f32,
    gain: f32,
}

impl AutoGain {
    const TARGET_RMS: f32 = 0.1; // -20 dBFS
    const MAX_GAIN: f32 = 8.0; // +18 dB
    /// Blocks quieter than this are pauses or room noise and don't move the gain.
    const SPEECH_FLOOR: f32 = 0.004; // about -48 dBFS
    /// Per ~10 ms block: settles within a few hundred milliseconds of speech.
    const SMOOTHING: f32 = 0.08;

    pub fn new() -> Self {
        Self { speech_rms: Self::TARGET_RMS, gain: 1.0 }
    }

    /// How loud this speaker's speech is, before gain: what "talking" looks like on this mic.
    pub fn speech_level(&self) -> f32 {
        self.speech_rms
    }

    pub fn process(&mut self, block: &mut [f32]) {
        if block.is_empty() {
            return;
        }
        let rms = (block.iter().map(|s| s * s).sum::<f32>() / block.len() as f32).sqrt();
        if rms > Self::SPEECH_FLOOR {
            self.speech_rms += (rms - self.speech_rms) * Self::SMOOTHING;
        }
        let target = (Self::TARGET_RMS / self.speech_rms.max(1e-6)).clamp(1.0, Self::MAX_GAIN);
        // Ramp across the block rather than stepping, which would click.
        let step = (target - self.gain) / block.len() as f32;
        for sample in block.iter_mut() {
            self.gain += step;
            *sample = soft_limit(*sample * self.gain);
        }
        self.gain = target;
    }
}

/// Linear up to 0.8, then eases into ±1 instead of clipping hard.
fn soft_limit(x: f32) -> f32 {
    const KNEE: f32 = 0.8;
    let a = x.abs();
    if a <= KNEE {
        x
    } else {
        let over = (a - KNEE) / (1.0 - KNEE);
        x.signum() * (KNEE + (1.0 - KNEE) * over.tanh())
    }
}

/// Why hands-free listening ended by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// The speaker talked, then went quiet.
    Silence,
    /// Nobody said anything.
    NoSpeech,
    TooLong,
}

/// Decides when hands-free listening is over, block by block, from the level before gain. What
/// counts as speech follows the auto-gain's estimate of this voice on this mic, so a quiet laptop
/// mic and a loud headset both work.
pub struct Endpointer {
    elapsed_ms: f32,
    last_speech_ms: Option<f32>,
}

impl Endpointer {
    /// Bounds on the speech threshold, in RMS before gain (about -48 to -38 dBFS).
    const FLOOR: f32 = 0.004;
    const CEILING: f32 = 0.012;

    pub fn new() -> Self {
        Self { elapsed_ms: 0.0, last_speech_ms: None }
    }

    pub fn threshold(speech_level: f32) -> f32 {
        (speech_level * 0.25).clamp(Self::FLOOR, Self::CEILING)
    }

    pub fn feed(&mut self, rms: f32, threshold: f32, block_ms: f32) -> Option<Ended> {
        self.elapsed_ms += block_ms;
        if rms > threshold {
            self.last_speech_ms = Some(self.elapsed_ms);
        }
        match self.last_speech_ms {
            Some(at) if self.elapsed_ms - at >= VOICE_END_SILENCE_MS as f32 => Some(Ended::Silence),
            None if self.elapsed_ms >= VOICE_NO_SPEECH_MS as f32 => Some(Ended::NoSpeech),
            _ if self.elapsed_ms >= (VOICE_MAX_SECONDS * 1000) as f32 => Some(Ended::TooLong),
            _ => None,
        }
    }
}

/// The short tones that tell someone who can't see the screen what state voice is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Earcon {
    /// Rising two notes: talk now.
    Listening,
    /// One soft note: got it, working on it.
    Thinking,
    /// Two quick even notes: Aloe asked you something, your turn.
    YourTurn,
    /// Falling two notes: stopped listening without anything to send.
    Stopped,
    /// One low note.
    Error,
}

/// The tone as 24 kHz samples, at `volume` (0–1). Each note has short fades so nothing clicks.
pub fn earcon(kind: Earcon, volume: f32) -> Vec<f32> {
    let notes: &[(f32, u32)] = match kind {
        Earcon::Listening => &[(660.0, 90), (0.0, 30), (880.0, 120)],
        Earcon::Thinking => &[(523.0, 110)],
        Earcon::YourTurn => &[(784.0, 90), (0.0, 60), (784.0, 90)],
        Earcon::Stopped => &[(880.0, 90), (0.0, 30), (587.0, 130)],
        Earcon::Error => &[(220.0, 260)],
    };
    let amplitude = 0.35 * volume.clamp(0.0, 1.0);
    let rate = VOICE_RATE as f32;
    let fade = (rate * 0.012) as usize;
    let mut out = Vec::new();
    for &(frequency, ms) in notes {
        let len = (rate * ms as f32 / 1000.0) as usize;
        for i in 0..len {
            if frequency == 0.0 {
                out.push(0.0);
                continue;
            }
            let envelope = (i.min(len - 1 - i) as f32 / fade as f32).min(1.0);
            out.push(amplitude * envelope * (std::f32::consts::TAU * frequency * i as f32 / rate).sin());
        }
    }
    out
}

pub fn read_level(meter: &AtomicU32) -> f32 {
    f32::from_bits(meter.load(Ordering::Relaxed))
}

pub fn to_pcm16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

/// Little-endian 16-bit PCM bytes, as they arrive over HTTP, possibly split mid-sample.
#[derive(Default)]
pub struct Pcm16Decoder {
    odd: Option<u8>,
}

impl Pcm16Decoder {
    pub fn push(&mut self, bytes: &[u8], out: &mut Vec<f32>) {
        let mut bytes = bytes.iter().copied();
        if let Some(low) = self.odd.take() {
            match bytes.next() {
                Some(high) => out.push(i16::from_le_bytes([low, high]) as f32 / 32768.0),
                None => {
                    self.odd = Some(low);
                    return;
                }
            }
        }
        loop {
            match (bytes.next(), bytes.next()) {
                (Some(low), Some(high)) => out.push(i16::from_le_bytes([low, high]) as f32 / 32768.0),
                (Some(low), None) => {
                    self.odd = Some(low);
                    return;
                }
                _ => return,
            }
        }
    }
}

/// A recording in progress. Dropping it (or `stop`) ends it; by the time `stop` returns, every
/// chunk it captured has been sent, and the sender is gone, so the receiver sees the end.
pub struct Capture {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    /// 0–1 loudness of the latest chunk, as f32 bits.
    level: Arc<AtomicU32>,
    stats: Arc<CaptureStats>,
    started: Instant,
}

/// What a recording actually got from the device, logged when it ends. Audio the device dropped
/// (an overrun) shows up as captured time falling short of the time the hold lasted.
#[derive(Default)]
struct CaptureStats {
    rate: AtomicU32,
    frames: std::sync::atomic::AtomicU64,
    overruns: AtomicU32,
    callbacks: AtomicU32,
    slowest_callback_us: AtomicU32,
}

impl Capture {
    /// Opens the default input device and starts sending 24 kHz PCM16 chunks to `chunks`. With
    /// `ended`, it also watches for the speaker to finish (`Endpointer`) and says so there, once;
    /// stopping is still the caller's job.
    pub fn start(chunks: UnboundedSender<Vec<i16>>, ended: Option<oneshot::Sender<Ended>>) -> Result<Capture, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let level = Arc::new(AtomicU32::new(0));
        let stats = Arc::new(CaptureStats::default());
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);
        let thread = {
            let (stop, level, stats) = (stop.clone(), level.clone(), stats.clone());
            std::thread::Builder::new()
                .name("aloe-voice-mic".into())
                .spawn(move || {
                    let stream = match open_input(chunks, level, stats, ended) {
                        Ok(stream) => stream,
                        Err(error) => {
                            let _ = ready_tx.send(Err(error));
                            return;
                        }
                    };
                    let _ = ready_tx.send(Ok(()));
                    while !stop.load(Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_millis(15));
                    }
                    drop(stream);
                })
                .map_err(|e| e.to_string())?
        };
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Capture { stop, thread: Some(thread), level, stats, started: Instant::now() }),
            Ok(Err(error)) => Err(error),
            Err(_) => Err("The microphone stopped before it started.".to_string()),
        }
    }

    /// The live loudness, readable from anywhere while the recording runs (see `read_level`).
    pub fn meter(&self) -> Arc<AtomicU32> {
        self.level.clone()
    }

    pub fn stop(mut self) {
        self.finish();
    }

    fn finish(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
            let s = &self.stats;
            let rate = s.rate.load(Ordering::Relaxed).max(1) as f64;
            debug_log(
                "voice",
                "mic_stats",
                format!(
                    "held={:.2}s captured={:.2}s overruns={} callbacks={} slowest_callback={}us",
                    self.started.elapsed().as_secs_f64(),
                    s.frames.load(Ordering::Relaxed) as f64 / rate,
                    s.overruns.load(Ordering::Relaxed),
                    s.callbacks.load(Ordering::Relaxed),
                    s.slowest_callback_us.load(Ordering::Relaxed),
                ),
            );
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.finish();
    }
}

fn open_input(chunks: UnboundedSender<Vec<i16>>, level: Arc<AtomicU32>, stats: Arc<CaptureStats>, ended: Option<oneshot::Sender<Ended>>) -> Result<cpal::Stream, String> {
    let device = cpal::default_host().default_input_device().ok_or_else(|| "No microphone found.".to_string())?;
    let supported = device.default_input_config().map_err(|e| format!("Couldn't open the microphone: {e}"))?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    debug_log("voice", "mic_config", format!("{format} {}Hz {}ch buffer={:?}", config.sample_rate, config.channels, config.buffer_size));
    stats.rate.store(config.sample_rate, Ordering::Relaxed);
    let stream = match format {
        SampleFormat::F32 => build_input::<f32>(&device, &config, chunks, level, stats, ended),
        SampleFormat::I16 => build_input::<i16>(&device, &config, chunks, level, stats, ended),
        SampleFormat::I32 => build_input::<i32>(&device, &config, chunks, level, stats, ended),
        SampleFormat::U16 => build_input::<u16>(&device, &config, chunks, level, stats, ended),
        SampleFormat::U8 => build_input::<u8>(&device, &config, chunks, level, stats, ended),
        other => return Err(format!("The microphone uses an unsupported sample format ({other}).")),
    }?;
    stream.play().map_err(|e| format!("Couldn't start the microphone: {e}"))?;
    Ok(stream)
}

fn build_input<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    chunks: UnboundedSender<Vec<i16>>,
    level: Arc<AtomicU32>,
    stats: Arc<CaptureStats>,
    mut ended: Option<oneshot::Sender<Ended>>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels.max(1) as usize;
    let mut resampler = Resampler::new(config.sample_rate, VOICE_RATE);
    let mut gain = AutoGain::new();
    let mut endpointer = Endpointer::new();
    let rate = config.sample_rate.max(1) as f32;
    let mut mono = Vec::new();
    let mut resampled = Vec::new();
    let error_stats = stats.clone();
    device
        .build_input_stream(
            config.clone(),
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                let began = Instant::now();
                mono.clear();
                for frame in data.chunks(channels) {
                    mono.push(frame.iter().map(|&s| f32::from_sample(s)).sum::<f32>() / frame.len() as f32);
                }
                if mono.is_empty() {
                    return;
                }
                if ended.is_some() {
                    let raw_rms = (mono.iter().map(|s| s * s).sum::<f32>() / mono.len() as f32).sqrt();
                    let block_ms = mono.len() as f32 * 1000.0 / rate;
                    if let Some(reason) = endpointer.feed(raw_rms, Endpointer::threshold(gain.speech_level()), block_ms) {
                        if let Some(ended) = ended.take() {
                            let _ = ended.send(reason);
                        }
                    }
                }
                gain.process(&mut mono);
                let rms = (mono.iter().map(|s| s * s).sum::<f32>() / mono.len() as f32).sqrt();
                // ×12 maps a normal speaking voice to most of the 0–1 range, as the composer's mic does.
                level.store((rms * 12.0).min(1.0).to_bits(), Ordering::Relaxed);
                resampled.clear();
                resampler.process(&mono, &mut resampled);
                if !resampled.is_empty() {
                    let _ = chunks.send(resampled.iter().map(|&s| to_pcm16(s)).collect());
                }
                stats.frames.fetch_add(mono.len() as u64, Ordering::Relaxed);
                stats.callbacks.fetch_add(1, Ordering::Relaxed);
                stats.slowest_callback_us.fetch_max(began.elapsed().as_micros() as u32, Ordering::Relaxed);
            },
            move |error| match error.kind() {
                cpal::ErrorKind::Xrun => {
                    error_stats.overruns.fetch_add(1, Ordering::Relaxed);
                }
                _ => debug_log("voice", "mic_stream_error", error.to_string()),
            },
            None,
        )
        .map_err(|e| format!("Couldn't open the microphone: {e}"))
}

/// Plays Aloe's reply: 24 kHz PCM pushed in as it downloads, played in order.
pub struct Player {
    queue: Arc<Mutex<VecDeque<f32>>>,
    resampler: Mutex<Resampler>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Player {
    pub fn start() -> Result<Player, String> {
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<u32, String>>(1);
        let thread = {
            let (queue, stop) = (queue.clone(), stop.clone());
            std::thread::Builder::new()
                .name("aloe-voice-speaker".into())
                .spawn(move || {
                    let (stream, rate) = match open_output(queue) {
                        Ok(opened) => opened,
                        Err(error) => {
                            let _ = ready_tx.send(Err(error));
                            return;
                        }
                    };
                    let _ = ready_tx.send(Ok(rate));
                    while !stop.load(Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_millis(15));
                    }
                    drop(stream);
                })
                .map_err(|e| e.to_string())?
        };
        match ready_rx.recv() {
            Ok(Ok(rate)) => Ok(Player { queue, resampler: Mutex::new(Resampler::new(VOICE_RATE, rate)), stop, thread: Some(thread) }),
            Ok(Err(error)) => Err(error),
            Err(_) => Err("The speaker stopped before it started.".to_string()),
        }
    }

    /// Queues 24 kHz mono samples.
    pub fn push(&self, samples: &[f32]) {
        let mut out = Vec::with_capacity(samples.len() * 2);
        self.resampler.lock().unwrap_or_else(std::sync::PoisonError::into_inner).process(samples, &mut out);
        self.queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner).extend(out);
    }

    /// Silences whatever is queued.
    pub fn clear(&self) {
        self.queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
    }

    pub fn is_idle(&self) -> bool {
        self.queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner).is_empty()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn open_output(queue: Arc<Mutex<VecDeque<f32>>>) -> Result<(cpal::Stream, u32), String> {
    let device = cpal::default_host().default_output_device().ok_or_else(|| "No speaker found.".to_string())?;
    let supported = device.default_output_config().map_err(|e| format!("Couldn't open the speaker: {e}"))?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let rate = config.sample_rate;
    let stream = match format {
        SampleFormat::F32 => build_output::<f32>(&device, &config, queue),
        SampleFormat::I16 => build_output::<i16>(&device, &config, queue),
        SampleFormat::I32 => build_output::<i32>(&device, &config, queue),
        SampleFormat::U16 => build_output::<u16>(&device, &config, queue),
        SampleFormat::U8 => build_output::<u8>(&device, &config, queue),
        other => return Err(format!("The speaker uses an unsupported sample format ({other}).")),
    }?;
    stream.play().map_err(|e| format!("Couldn't start the speaker: {e}"))?;
    Ok((stream, rate))
}

fn build_output<T>(device: &cpal::Device, config: &StreamConfig, queue: Arc<Mutex<VecDeque<f32>>>) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels.max(1) as usize;
    device
        .build_output_stream(
            config.clone(),
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                let mut queue = queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                for frame in data.chunks_mut(channels) {
                    let sample = T::from_sample(queue.pop_front().unwrap_or(0.0));
                    frame.fill(sample);
                }
            },
            |error| debug_log("voice", "speaker_stream_error", error.to_string()),
            None,
        )
        .map_err(|e| format!("Couldn't open the speaker: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_halves_48k_across_chunk_boundaries() {
        let mut r = Resampler::new(48_000, 24_000);
        let input: Vec<f32> = (0..960).map(|i| i as f32).collect();
        let mut out = Vec::new();
        for chunk in input.chunks(97) {
            r.process(chunk, &mut out);
        }
        // Every second input sample, with nothing dropped or doubled at the seams.
        assert!((out.len() as i64 - 480).abs() <= 1, "got {}", out.len());
        for (i, &s) in out.iter().enumerate() {
            assert!((s - (i * 2) as f32).abs() < 1e-3, "sample {i} = {s}");
        }
    }

    #[test]
    fn resampler_upsamples_by_interpolating() {
        let mut r = Resampler::new(24_000, 48_000);
        let mut out = Vec::new();
        r.process(&[0.0, 1.0], &mut out);
        r.process(&[2.0], &mut out);
        assert_eq!(out, vec![0.0, 0.5, 1.0, 1.5]);
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    fn tone(amplitude: f32, len: usize) -> Vec<f32> {
        (0..len).map(|i| amplitude * (i as f32 * 0.07).sin()).collect()
    }

    #[test]
    fn auto_gain_brings_quiet_speech_up_without_clipping() {
        let mut agc = AutoGain::new();
        let mut last = Vec::new();
        // ~-38 dBFS speech, as measured from a real laptop mic, for two seconds of 10 ms blocks.
        for _ in 0..200 {
            last = tone(0.018, 480);
            agc.process(&mut last);
        }
        let level = rms(&last);
        assert!(level > 0.08 && level < 0.12, "speech settled at rms {level}");
        assert!(last.iter().all(|s| s.abs() < 1.0));
    }

    #[test]
    fn auto_gain_leaves_loud_speech_alone_and_never_pumps_silence() {
        let mut agc = AutoGain::new();
        let mut loud = tone(0.3, 480);
        let before = loud.clone();
        agc.process(&mut loud);
        assert!(loud.iter().zip(&before).all(|(a, b)| (a - b).abs() < 0.11), "loud input is not boosted, only limited");

        let mut agc = AutoGain::new();
        let mut hush = Vec::new();
        for _ in 0..200 {
            hush = tone(0.001, 480);
            agc.process(&mut hush);
        }
        assert!(rms(&hush) < 0.002, "room noise between words stays where it was");
    }

    #[test]
    fn endpointer_waits_for_speech_then_for_the_pause_after_it() {
        let mut e = Endpointer::new();
        let threshold = 0.01;
        // Half a second of quiet, a second of talking with short gaps between words.
        for _ in 0..50 {
            assert_eq!(e.feed(0.0, threshold, 10.0), None);
        }
        for i in 0..100 {
            assert_eq!(e.feed(if i % 20 < 15 { 0.05 } else { 0.0 }, threshold, 10.0), None);
        }
        // A pause shorter than the end-of-speech silence doesn't end it (the last word was 50ms
        // before the end of the loop above, so this stops 50ms short of the limit).
        for _ in 0..(VOICE_END_SILENCE_MS / 10 - 10) {
            assert_eq!(e.feed(0.0, threshold, 10.0), None);
        }
        assert_eq!(e.feed(0.05, threshold, 10.0), None);
        let mut ended = None;
        for _ in 0..(VOICE_END_SILENCE_MS / 10 + 1) {
            ended = ended.or(e.feed(0.0, threshold, 10.0));
        }
        assert_eq!(ended, Some(Ended::Silence));
    }

    #[test]
    fn endpointer_gives_up_when_nobody_talks() {
        let mut e = Endpointer::new();
        let mut ended = None;
        for _ in 0..(VOICE_NO_SPEECH_MS / 10 + 1) {
            ended = ended.or(e.feed(0.002, 0.004, 10.0));
        }
        assert_eq!(ended, Some(Ended::NoSpeech));
    }

    #[test]
    fn endpointer_threshold_follows_the_voice_within_bounds() {
        assert_eq!(Endpointer::threshold(0.0126), 0.004_f32.max(0.0126 * 0.25));
        assert_eq!(Endpointer::threshold(0.5), 0.012);
        assert_eq!(Endpointer::threshold(0.0001), 0.004);
    }

    #[test]
    fn earcons_are_short_quiet_and_click_free() {
        for kind in [Earcon::Listening, Earcon::Thinking, Earcon::YourTurn, Earcon::Stopped, Earcon::Error] {
            let tone = earcon(kind, 1.0);
            assert!(!tone.is_empty() && tone.len() < VOICE_RATE as usize / 2, "{kind:?} under half a second");
            assert!(tone.iter().all(|s| s.abs() <= 0.35 + 1e-6));
            assert!(tone[0].abs() < 0.01 && tone[tone.len() - 1].abs() < 0.05, "{kind:?} fades in and out");
        }
        assert!(earcon(Earcon::Thinking, 0.0).iter().all(|s| *s == 0.0));
    }

    #[test]
    fn pcm16_decoder_survives_odd_splits() {
        let samples: [i16; 3] = [1000, -2000, 32767];
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut decoder = Pcm16Decoder::default();
        let mut out = Vec::new();
        decoder.push(&bytes[..1], &mut out);
        decoder.push(&bytes[1..4], &mut out);
        decoder.push(&bytes[4..], &mut out);
        let back: Vec<i16> = out.iter().map(|s| (s * 32768.0) as i16).collect();
        assert_eq!(back, samples);
    }
}
