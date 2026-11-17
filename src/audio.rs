//! Live audio sources: the microphone, or an audio file played through the
//! speakers. Either way, samples go through a lock-free ring buffer to an
//! analysis thread, which sends finished feature frames to Bevy.
//!
//! ```text
//! cpal callback ──ringbuf──▶ analysis thread ──mpsc──▶ Bevy (receive_live)
//! ```
//!
//! The cpal callback only copies samples (as left/right pairs; mono sources
//! send the same sample on both sides): no allocation, no locks, no logging.
//! For a file, the samples analysed are exactly the ones being played, so the
//! visuals stay in sync with what you hear (and the mic isn't involved).

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use bevy::prelude::*;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::{
    HeapRb,
    traits::{Consumer, Producer, Split},
};

use crate::{
    analysis::Analyzer,
    features::{AudioFeatures, FeatureSource},
};

/// What the live source is.
#[derive(Debug, Clone)]
pub enum LiveSpec {
    Mic { device: Option<String> },
    /// Whatever the computer is playing, tapped before it reaches the speakers.
    System,
    File { path: PathBuf, mute: bool },
}

/// A running live source. Dropping it stops the audio and analysis threads.
#[derive(Resource)]
pub struct LiveAudio {
    pub label: String,
    // Mutex only to make the resource Sync; it's never contended.
    frames: Mutex<mpsc::Receiver<AudioFeatures>>,
    stop: Arc<AtomicBool>,
}

impl Drop for LiveAudio {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreStartup, start_live)
            .add_systems(Update, receive_live.before(crate::features::FeatureSet));
    }
}

fn start_live(mut commands: Commands, cfg: Res<crate::config::Config>, mut source: ResMut<FeatureSource>) {
    let Some(spec) = cfg.live.clone() else {
        return;
    };
    match LiveAudio::start(spec) {
        Ok(live) => {
            info!("live audio: {}", live.label);
            commands.insert_resource(live);
            *source = FeatureSource::Live;
        }
        Err(e) => {
            warn!("no live audio ({e}); using the synthetic source");
            *source = FeatureSource::Synthetic;
        }
    }
}

/// Take every frame analysed since last time: keep the newest, sum the onsets.
fn receive_live(live: Option<Res<LiveAudio>>, source: Res<FeatureSource>, mut features: ResMut<AudioFeatures>) {
    let Some(live) = live else { return };
    let mut latest: Option<AudioFeatures> = None;
    // Several analysis frames can arrive per display frame: keep the latest
    // readings, but count every event.
    let (mut onsets, mut beats, mut strength) = (0, 0, 0.0f32);
    let frames = live.frames.lock().expect("frame receiver poisoned");
    while let Ok(f) = frames.try_recv() {
        onsets += f.onsets;
        beats += f.beats;
        strength = strength.max(f.beat_strength);
        latest = Some(f);
    }
    if *source != FeatureSource::Live {
        return;
    }
    if let Some(mut f) = latest {
        f.onsets = onsets;
        f.beats = beats;
        f.beat_strength = strength;
        *features = f;
    } else {
        features.clear_events();
    }
}

/// Print the input devices, for `--list-devices`.
pub fn list_input_devices() {
    let host = cpal::default_host();
    let default = host.default_input_device().and_then(|d| d.description().ok()).map(|d| d.name().to_string());
    match host.input_devices() {
        Ok(devices) => {
            println!("Input devices:");
            for d in devices {
                let name = d.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "?".into());
                let mark = if Some(&name) == default.as_ref() { "  (default)" } else { "" };
                println!("  {name}{mark}");
            }
        }
        Err(e) => println!("cannot list input devices: {e}"),
    }
}

pub type AnyError = Box<dyn std::error::Error + Send + Sync>;

impl LiveAudio {
    pub fn start(spec: LiveSpec) -> Result<Self, AnyError> {
        let stop = Arc::new(AtomicBool::new(false));
        let (frame_tx, frames) = mpsc::channel();
        // The stream thread builds the cpal stream (which must stay on the thread
        // that made it) and reports back the label and sample rate, or an error.
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(String, u32), String>>(1);
        // One second of headroom between audio callback and analysis.
        let rb = HeapRb::<Stereo>::new(96_000);
        let (producer, consumer) = rb.split();

        let stop_stream = stop.clone();
        thread::Builder::new().name("audio-stream".into()).spawn(move || {
            let result = match &spec {
                LiveSpec::Mic { device } => mic_stream(device.as_deref(), producer),
                LiveSpec::System => system_stream(producer),
                LiveSpec::File { path, mute } => file_stream(path, *mute, producer),
            };
            match result {
                Ok((stream, label, rate)) => {
                    if let Err(e) = stream.play() {
                        let _ = ready_tx.send(Err(e.to_string()));
                        return;
                    }
                    let _ = ready_tx.send(Ok((label, rate)));
                    while !stop_stream.load(Ordering::Relaxed) {
                        thread::sleep(Duration::from_millis(50));
                    }
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e.to_string()));
                }
            }
        })?;

        let (label, rate) = ready_rx
            .recv_timeout(Duration::from_secs(20))
            .map_err(|_| "audio stream did not start")??;

        let stop_analysis = stop.clone();
        thread::Builder::new().name("audio-analysis".into()).spawn(move || {
            let mut consumer = consumer;
            let mut analyzer = Analyzer::new(rate as f32);
            let mut buf = vec![[0.0f32; 2]; 2048];
            while !stop_analysis.load(Ordering::Relaxed) {
                let n = consumer.pop_slice(&mut buf);
                if n == 0 {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                }
                analyzer.push_stereo(buf[..n].as_flattened(), |f, _| {
                    let _ = frame_tx.send(f.clone());
                });
            }
        })?;

        Ok(Self { label, frames: Mutex::new(frames), stop })
    }
}

/// One sample frame: left, right.
type Stereo = [f32; 2];
type Prod = <HeapRb<Stereo> as Split>::Prod;

#[cfg(target_os = "linux")]
/// Capture the monitor of the default output (PulseAudio/PipeWire): the audio
/// being played, before it is emitted.
fn system_stream(producer: Prod) -> Result<(cpal::Stream, String, u32), AnyError> {
    let host = cpal::host_from_id(cpal::HostId::PulseAudio)
        .map_err(|e| format!("system audio needs PulseAudio or PipeWire: {e}"))?;
    let default_out = host.default_output_device().and_then(|d| d.id().ok()).map(|id| id.to_string());
    let monitors: Vec<_> = host
        .input_devices()?
        .filter(|d| d.id().is_ok_and(|id| id.to_string().ends_with(".monitor")))
        .collect();
    let device = monitors
        .iter()
        .find(|d| {
            let id = d.id().map(|id| id.to_string()).unwrap_or_default();
            default_out.as_ref().is_some_and(|out| id.strip_suffix(".monitor") == Some(out.as_str()))
        })
        .or(monitors.first())
        .cloned()
        .ok_or("no monitor source found (is the sound server running?)")?;
    input_stream(device, "system", producer, Direction::Input)
}

#[cfg(target_os = "macos")]
/// Tap the default output device (Core Audio process tap, macOS 14.6+): cpal
/// does this when an input stream is opened on an output device.
fn system_stream(producer: Prod) -> Result<(cpal::Stream, String, u32), AnyError> {
    let device = cpal::default_host().default_output_device().ok_or("no default output device")?;
    input_stream(device, "system", producer, Direction::Output)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn system_stream(_producer: Prod) -> Result<(cpal::Stream, String, u32), AnyError> {
    Err("--system is only supported on Linux and macOS".into())
}

/// macOS only: make sure we may record system audio before the window opens.
/// If macOS has never asked, a short tap brings up its prompt and we wait for
/// the answer. If the answer is no, this returns the instructions (and the
/// decision is reset so the next run asks again).
#[cfg(target_os = "macos")]
pub fn check_system_permission() -> Result<(), String> {
    use crate::system_audio_permission::{Access, access, denied_message, wait_for_answer};
    match access() {
        Access::Granted => return Ok(()),
        Access::Denied => return Err(denied_message()),
        Access::Unknown => {}
    }
    eprintln!("synestesia: macOS will ask to let this app record system audio; choose Allow.");
    let rb = HeapRb::<Stereo>::new(4096);
    let (producer, _consumer) = rb.split();
    let (stream, ..) = system_stream(producer).map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    let answer = wait_for_answer(Duration::from_secs(60));
    drop(stream);
    match answer {
        Access::Granted => Ok(()),
        Access::Denied => Err(denied_message()),
        // No answer, or no prompt at all (some launchers can't show one).
        Access::Unknown => Err(format!("no answer to the system audio prompt.\n\n{}", denied_message())),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn check_system_permission() -> Result<(), String> {
    Ok(())
}

#[derive(Clone, Copy)]
enum Direction {
    Input,
    #[allow(dead_code)]
    Output,
}

fn mic_stream(device: Option<&str>, producer: Prod) -> Result<(cpal::Stream, String, u32), AnyError> {
    let host = cpal::default_host();
    let device = match device {
        None => host.default_input_device().ok_or("no default input device")?,
        Some(want) => {
            let want = want.to_lowercase();
            host.input_devices()?
                .find(|d| d.description().is_ok_and(|desc| desc.name().to_lowercase().contains(&want)))
                .ok_or_else(|| format!("no input device matching \"{want}\" (try --list-devices)"))?
        }
    };
    input_stream(device, "mic", producer, Direction::Input)
}

fn input_stream(
    device: cpal::Device,
    kind: &str,
    mut producer: Prod,
    direction: Direction,
) -> Result<(cpal::Stream, String, u32), AnyError> {
    let name = device.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "input".into());
    // A tapped output device has no input format of its own: record in its output format.
    let supported = match direction {
        Direction::Input => device.default_input_config()?,
        Direction::Output => device.default_output_config()?,
    };
    let config: cpal::StreamConfig = supported.config();
    let channels = config.channels as usize;
    let rate = config.sample_rate;
    let err = |e| eprintln!("audio input error: {e}");

    // As left/right pairs, in small fixed chunks (no allocation).
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(
            config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| push_stereo(data, channels, &mut producer, |s| s),
            err,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_input_stream(
            config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                push_stereo(data, channels, &mut producer, |s| s as f32 / 32768.0)
            },
            err,
            None,
        )?,
        cpal::SampleFormat::I32 => device.build_input_stream(
            config,
            move |data: &[i32], _: &cpal::InputCallbackInfo| {
                push_stereo(data, channels, &mut producer, |s| s as f32 / 2_147_483_648.0)
            },
            err,
            None,
        )?,
        other => return Err(format!("unsupported input sample format {other}").into()),
    };
    Ok((stream, format!("{kind}: {name} ({rate} Hz)"), rate))
}

/// The first two channels as left and right (a mono input on both).
#[inline]
fn push_stereo<T: Copy>(data: &[T], channels: usize, producer: &mut Prod, to_f32: impl Fn(T) -> f32) {
    let mut chunk = [[0.0f32; 2]; 256];
    let mut n = 0;
    for frame in data.chunks_exact(channels) {
        let l = to_f32(frame[0]);
        let r = if channels > 1 { to_f32(frame[1]) } else { l };
        chunk[n] = [l, r];
        n += 1;
        if n == chunk.len() {
            producer.push_slice(&chunk);
            n = 0;
        }
    }
    producer.push_slice(&chunk[..n]);
}

/// Decode a whole file into interleaved f32, returning (samples, channels, rate).
pub fn decode_file(path: &Path) -> Result<(Vec<f32>, usize, u32), AnyError> {
    use symphonia::core::{
        codecs::audio::AudioDecoderOptions,
        errors::Error,
        formats::{FormatOptions, TrackType, probe::Hint},
        io::MediaSourceStream,
        meta::MetadataOptions,
    };

    let file = Box::new(
        std::fs::File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?,
    );
    let mss = MediaSourceStream::new(file, Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe().probe(
        &hint,
        mss,
        FormatOptions::default(),
        MetadataOptions::default(),
    )?;
    let track = format.default_track(TrackType::Audio).ok_or("no audio track")?;
    let params = track.codec_params.as_ref().and_then(|p| p.audio()).ok_or("no audio codec parameters")?;
    let mut decoder = symphonia::default::get_codecs().make_audio_decoder(params, &AudioDecoderOptions::default())?;
    let track_id = track.id;

    let mut out = Vec::new();
    let mut chunk = Vec::new();
    let (mut channels, mut rate) = (0usize, 0u32);
    while let Some(packet) = format.next_packet()? {
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                channels = buf.spec().channels().count();
                rate = buf.spec().rate();
                chunk.resize(buf.samples_interleaved(), 0.0f32);
                buf.copy_to_slice_interleaved(&mut chunk);
                out.extend_from_slice(&chunk);
            }
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    if out.is_empty() || channels == 0 || rate == 0 {
        return Err("file decoded to no audio".into());
    }
    Ok((out, channels, rate))
}

/// Play a decoded file on the default output device, looping, and push the
/// played signal (left and right) to the analyser. Resamples linearly if the device rate
/// differs from the file's.
fn file_stream(path: &Path, mute: bool, mut producer: Prod) -> Result<(cpal::Stream, String, u32), AnyError> {
    let (samples, file_ch, file_rate) = decode_file(path)?;
    let frames = samples.len() / file_ch;

    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("no default output device")?;
    let supported = device.default_output_config()?;
    if supported.sample_format() != cpal::SampleFormat::F32 {
        return Err(format!("output device wants {}, only f32 is supported", supported.sample_format()).into());
    }
    let config: cpal::StreamConfig = supported.config();
    let out_ch = config.channels as usize;
    let out_rate = config.sample_rate;
    let step = file_rate as f64 / out_rate as f64;
    let mut pos = 0.0f64;

    let stream = device.build_output_stream(
        config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let mut pairs = [[0.0f32; 2]; 256];
            let mut n = 0;
            for frame in data.chunks_exact_mut(out_ch) {
                let i = pos as usize;
                let frac = (pos - i as f64) as f32;
                let j = (i + 1) % frames;
                let sample = |ch: usize| {
                    let c = ch.min(file_ch - 1);
                    let a = samples[i * file_ch + c];
                    let b = samples[j * file_ch + c];
                    a + (b - a) * frac
                };
                let (l, r) = (sample(0), sample(1));
                for (k, o) in frame.iter_mut().enumerate() {
                    *o = if mute { 0.0 } else if k % 2 == 0 { l } else { r };
                }
                pairs[n] = [l, r];
                n += 1;
                if n == pairs.len() {
                    producer.push_slice(&pairs);
                    n = 0;
                }
                pos += step;
                if pos >= frames as f64 {
                    pos -= frames as f64;
                }
            }
            producer.push_slice(&pairs[..n]);
        },
        |e| eprintln!("audio output error: {e}"),
        None,
    )?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let secs = frames as f32 / file_rate as f32;
    let muted = if mute { ", muted" } else { "" };
    Ok((stream, format!("file: {name} ({secs:.0} s, {file_rate} Hz{muted})"), out_rate))
}
