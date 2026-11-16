//! `--analyze FILE…`: run the analysis on whole tracks, offline and faster than
//! real time, and write what the visuals would see, for `analysis/index.html`:
//!
//! - the raw spectrum (dB) and the normalised one the visuals use,
//! - every signal (bands, loudness, brightness, onsets, bass, activity, gust),
//! - onset and bass-hit events with their strengths,
//! - what each movement target would do with the default tuning.
//!
//! Each track becomes `analysis/<name>.js` (data) plus a link to the audio, so
//! the timeline can play the track with a moving playhead. A short summary is
//! printed too.

use std::{fmt::Write as _, path::Path};

use crate::{
    analysis::{Analyzer, HOP},
    features::SPECTRUM_BINS,
    tuning::{Control, Signal, Signals, Transformation, Transforms, Tuning},
};

/// Keep every n-th hop in the data (hops are ~11 ms; 2 → ~23 ms per column).
const DECIMATE: usize = 2;

fn q(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn db01(db: f32, lo: f32, hi: f32) -> f32 {
    (db - lo) / (hi - lo)
}

fn b64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

pub fn run(files: &[String], out_dir: &Path) {
    if let Err(e) = std::fs::create_dir_all(out_dir) {
        eprintln!("cannot create {}: {e}", out_dir.display());
        std::process::exit(1);
    }
    let mut names = Vec::new();
    for file in files {
        match analyze(Path::new(file), out_dir) {
            Ok(name) => names.push(name),
            Err(e) => eprintln!("{file}: {e}"),
        }
    }
    // The viewer reads the list of tracks from here.
    let list = names.iter().map(|n| format!("{n:?}")).collect::<Vec<_>>().join(", ");
    let _ = std::fs::write(out_dir.join("tracks.js"), format!("window.TRACKS = [{list}];\n"));
}

/// Display range of a control in the timeline.
fn view_range(c: Control) -> (f32, f32) {
    let spec = c.spec();
    if spec.clamp.1 - spec.clamp.0 > 100.0 { spec.base_range } else { spec.clamp }
}

fn analyze(path: &Path, out_dir: &Path) -> Result<String, crate::audio::AnyError> {
    let name = path.file_stem().and_then(|s| s.to_str()).ok_or("bad file name")?.to_string();
    let (samples, channels, rate) = crate::audio::decode_file(path)?;
    let mono: Vec<f32> = samples.chunks(channels).map(|c| c.iter().sum::<f32>() / channels as f32).collect();
    let dt = HOP as f32 / rate as f32;

    let tuning = Tuning::default();
    let mut analyzer = Analyzer::new(rate as f32);
    let mut transforms = Transforms::default();

    // Lanes: raw readings, then every signal, transform and control.
    let mut keys: Vec<String> = ["level_db", "bass_db", "low_mids_db", "high_mids_db", "treble_db", "flux", "tempo_bpm"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let first_signal = keys.len();
    keys.extend(Signal::ALL.map(|s| format!("s_{}", s.key())));
    let first_transform = keys.len();
    keys.extend(Transformation::ALL.map(|t| format!("t_{}", t.spec().key)));
    let first_control = keys.len();
    keys.extend(Control::ALL.map(|c| format!("c_{}", c.spec().key)));
    let mut lanes: Vec<Vec<u8>> = vec![Vec::new(); keys.len()];
    let mut sums = vec![0.0f64; keys.len()];
    let mut saturated = vec![0usize; keys.len()];
    let mut spec_raw = Vec::new();
    let mut spec_norm = Vec::new();
    let mut onsets: Vec<(f32, f32)> = Vec::new();
    let mut beats: Vec<(f32, f32)> = Vec::new();
    let mut flux_raw: Vec<f32> = Vec::new();
    let mut tempos: Vec<f32> = Vec::new();
    let mut vortex_wins = [0usize; 4];
    let mut hop = 0usize;
    let mut kept = 0usize;

    for chunk in mono.chunks(HOP) {
        analyzer.push(chunk, |f, raw| {
            let t = hop as f32 * dt;
            if raw.onset {
                onsets.push((t, raw.onset_strength));
            }
            if raw.beat {
                beats.push((t, raw.beat_strength));
            }
            let signals = Signals::from_features(f);
            transforms.step(&tuning, &signals, dt);
            hop += 1;
            if hop % DECIMATE != 0 {
                return;
            }
            let mut values = vec![
                db01(raw.level_db, -80.0, 0.0),
                db01(raw.bands_db[0], -90.0, 0.0),
                db01(raw.bands_db[1], -90.0, 0.0),
                db01(raw.bands_db[2], -90.0, 0.0),
                db01(raw.bands_db[3], -90.0, 0.0),
                0.0, // flux: filled in after the pass, relative to the track
                (raw.tempo_bpm - 60.0) / 120.0,
            ];
            values.extend(signals.0);
            values.extend(transforms.0);
            let controls = Control::ALL.map(|c| tuning.control(c, &transforms));
            values.extend(Control::ALL.iter().zip(controls).map(|(&c, v)| {
                let (lo, hi) = view_range(c);
                (v - lo) / (hi - lo)
            }));
            for (k, &v) in values.iter().enumerate() {
                lanes[k].push(q(v));
                sums[k] += v as f64;
                saturated[k] += (v > 0.9) as usize;
            }
            flux_raw.push(raw.flux);
            tempos.push(raw.tempo_bpm);
            for (&db, &n) in raw.spectrum_db.iter().zip(&f.spectrum) {
                spec_raw.push(q(db01(db, -90.0, 0.0)));
                spec_norm.push(q(n));
            }
            let vortices = Control::VORTICES.map(|c| tuning.control(c, &transforms));
            if vortices.iter().any(|&v| v > 0.05) {
                vortex_wins[(0..4).max_by(|&a, &b| vortices[a].total_cmp(&vortices[b])).unwrap()] += 1;
            }
            kept += 1;
        });
    }
    // Flux has no natural scale: show it relative to the track's 99th percentile.
    let mut sorted = flux_raw.clone();
    sorted.sort_by(f32::total_cmp);
    let p99 = sorted.get(sorted.len() * 99 / 100).copied().unwrap_or(1.0).max(1e-9);
    lanes[5] = flux_raw.iter().map(|&v| q(v / p99)).collect();

    // --- Audio the viewer can play: a link to the file, or for FLAC (which the
    // browser can't stream from a simple server) an AAC copy made by macOS's
    // `afconvert`, when available.
    let duration = mono.len() as f32 / rate as f32;
    let mut audio_name = path.file_name().and_then(|s| s.to_str()).unwrap_or_default().to_string();
    let is_flac = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("flac"));
    let link = out_dir.join(&audio_name);
    let _ = std::fs::remove_file(&link);
    let converted = is_flac
        && std::process::Command::new("afconvert")
            .args(["-f", "m4af", "-d", "aac", "-b", "192000"])
            .arg(path)
            .arg(out_dir.join(format!("{name}.play.m4a")))
            .status()
            .is_ok_and(|s| s.success());
    if converted {
        audio_name = format!("{name}.play.m4a");
    } else {
        let target = std::fs::canonicalize(path)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link)?;
    }

    // --- Data file for the viewer.
    let mut js = String::new();
    let _ = write!(
        js,
        "window.TRACK_DATA = window.TRACK_DATA || {{}};\nwindow.TRACK_DATA[{name:?}] = {{\n  audio: {audio_name:?},\n  duration: {duration},\n  frameDt: {},\n  frames: {kept},\n  bins: {SPECTRUM_BINS},\n",
        dt * DECIMATE as f32
    );
    let _ = writeln!(js, "  specRaw: {:?},\n  specNorm: {:?},", b64(&spec_raw), b64(&spec_norm));
    js += "  lanes: {\n";
    for (key, lane) in keys.iter().zip(&lanes) {
        let _ = writeln!(js, "    {key}: {:?},", b64(lane));
    }
    js += "  },\n";
    let ev = |v: &[(f32, f32)]| v.iter().map(|(t, s)| format!("[{t:.3},{s:.2}]")).collect::<Vec<_>>().join(",");
    let _ = writeln!(js, "  onsets: [{}],\n  beats: [{}],\n}};", ev(&onsets), ev(&beats));
    std::fs::write(out_dir.join(format!("{name}.js")), js)?;

    // --- Summary.
    let pct = |n: usize| 100.0 * n as f32 / kept.max(1) as f32;
    let mean = |k: usize| sums[k] / kept.max(1) as f64;
    let mins = duration / 60.0;
    tempos.sort_by(f32::total_cmp);
    let tempo = tempos.get(tempos.len() / 2).copied().unwrap_or(0.0);
    println!("\n== {name}: {:.0}:{:02.0}, {rate} Hz, {channels} ch", (duration / 60.0).floor(), duration % 60.0);
    println!(
        "  onsets {:.0}/min, beats {:.0}/min, median tempo {tempo:.0} BPM, pulse clarity {:.2}",
        onsets.len() as f32 / mins,
        beats.len() as f32 / mins,
        mean(first_signal + Signal::PulseClarity as usize)
    );
    let line = |title: &str, range: std::ops::Range<usize>, names: Vec<&str>| {
        let parts: Vec<String> = range.zip(names).map(|(k, n)| format!("{n} {:.2}/{:.0}%", mean(k), pct(saturated[k]))).collect();
        println!("  {title} (mean / time above 0.9): {}", parts.join("  "));
    };
    line("signals", first_signal..first_transform, Signal::ALL.map(|s| s.key()).to_vec());
    line("transforms", first_transform..first_control, Transformation::ALL.map(|t| t.spec().key).to_vec());
    line("controls", first_control..keys.len(), Control::ALL.map(|c| c.spec().key).to_vec());
    println!(
        "  strongest vortex (time share): large {:.0}%  medium {:.0}%  small {:.0}%  tiny {:.0}%",
        pct(vortex_wins[0]),
        pct(vortex_wins[1]),
        pct(vortex_wins[2]),
        pct(vortex_wins[3])
    );
    Ok(name)
}
