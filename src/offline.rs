//! `--analyze FILE…`: run the analysis on whole tracks, offline and faster than
//! real time, and write what the visuals would see, for `analysis/index.html`:
//!
//! - the raw spectrum (dB) and the normalised one the visuals use,
//! - every signal (bands, loudness, brightness, onsets, bass, activity, gust),
//! - onset and bass-hit events with their strengths,
//! - what each scene's controls would do with the default tuning,
//! - how far the glass window's panes would move (its real motion, run on a
//!   1920×1080 window).
//!
//! Each track becomes `analysis/<name>.js` (data) plus a link to the audio, so
//! the timeline can play the track with a moving playhead. A short summary is
//! printed too.

use std::{fmt::Write as _, path::Path};

use crate::{
    analysis::{Analyzer, HOP},
    features::SPECTRUM_BINS,
    bubble_controls::Control,
    glass_motion::{Drive, MAX_OFFSET, Motion},
    scene::{ControlId, Scene},
    tuning::{Signal, Signals, Transformation, Transforms, Tuning},
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
fn view_range(c: ControlId) -> (f32, f32) {
    let spec = c.spec();
    if spec.clamp.1 - spec.clamp.0 > 100.0 { spec.base_range } else { spec.clamp }
}

fn analyze(path: &Path, out_dir: &Path) -> Result<String, crate::audio::AnyError> {
    let name = path.file_stem().and_then(|s| s.to_str()).ok_or("bad file name")?.to_string();
    let (samples, channels, rate) = crate::audio::decode_file(path)?;
    // Left/right pairs (mono files on both sides).
    let stereo: Vec<f32> = samples.chunks(channels).flat_map(|c| [c[0], c[if channels > 1 { 1 } else { 0 }]]).collect();
    let frames_total = stereo.len() / 2;
    let dt = HOP as f32 / rate as f32;

    let tuning = Tuning::default();
    let mut analyzer = Analyzer::new(rate as f32);
    let mut transforms = Transforms::default();

    // Lanes: raw readings, then every signal, transform and control.
    let mut keys: Vec<String> = ["level_db", "bass_db", "low_mids_db", "high_mids_db", "treble_db", "flux", "tempo_bpm", "lead_hz"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let first_signal = keys.len();
    keys.extend(Signal::ALL.map(|s| format!("s_{}", s.key())));
    let first_transform = keys.len();
    keys.extend(Transformation::ALL.map(|t| format!("t_{}", t.spec().key)));
    let first_control = keys.len();
    let controls: Vec<ControlId> = Scene::ALL.into_iter().flat_map(Scene::controls).collect();
    keys.extend(controls.iter().map(|c| format!("c_{}_{}", c.scene.key(), c.spec().key)));
    keys.push("g_glass_offset".into());
    let mut lanes: Vec<Vec<u8>> = vec![Vec::new(); keys.len()];
    let mut sums = vec![0.0f64; keys.len()];
    let mut saturated = vec![0usize; keys.len()];
    let mut spec_raw = Vec::new();
    let mut spec_norm = Vec::new();
    let mut spec_lead_db: Vec<f32> = Vec::new();
    let mut onsets: Vec<(f32, f32)> = Vec::new();
    let mut beats: Vec<(f32, f32)> = Vec::new();
    let mut flux_raw: Vec<f32> = Vec::new();
    let mut tempos: Vec<f32> = Vec::new();
    let mut vortex_wins = [0usize; 4];
    // The glass window, moving.
    let screen = bevy::math::Vec2::new(1920.0, 1080.0);
    let mut panes = crate::glass::Panes::layout(90, screen);
    let mut motion = Motion::default();
    let mut rng = <rand::rngs::StdRng as rand::SeedableRng>::seed_from_u64(1);
    let spring = crate::glass_motion::spring(&tuning);
    let mut offsets: Vec<f32> = Vec::new();
    // Musical time, as the app keeps it (the beat kick and the dance count).
    let mut clock = crate::tuning::MusicClock::default();
    let (mut last_one, mut one_moves) = (0usize, 0usize);
    let mut light = crate::glass_light::Light::default();
    // The lead line: hops with a pitch, and note lengths.
    let (mut voiced_hops, mut notes, mut note_hops, mut last_hz) = (0usize, 0usize, 0usize, 0.0f32);
    let (mut voice_hops, mut rap_hops, mut rap_entries, mut heard_hops) = (0usize, 0usize, 0usize, 0usize);
    let (mut mosh_hops, mut flat_sum, mut last_drop, mut drop_times) = (0usize, 0.0f64, 0.0f32, Vec::<String>::new());
    let mut flats: Vec<f32> = Vec::new();
    let mut parts_sum = [0.0f64; 3];
    let mut hop = 0usize;
    let mut kept = 0usize;

    for chunk in stereo.chunks(2 * HOP) {
        analyzer.push_stereo(chunk, |f, raw| {
            let t = hop as f32 * dt;
            if raw.onset {
                onsets.push((t, raw.onset_strength));
            }
            if raw.beat {
                beats.push((t, raw.beat_strength));
            }
            if raw.melody_hz > 0.0 {
                voiced_hops += 1;
                note_hops += 1;
                if last_hz == 0.0 || (raw.melody_hz / last_hz).log2().abs() > 1.0 / 12.0 {
                    notes += 1;
                }
            }
            last_hz = raw.melody_hz;
            if raw.level_db > -50.0 {
                heard_hops += 1;
                voice_hops += (f.singing > 0.5) as usize;
                rap_hops += (f.rapping > 0.5) as usize;
                rap_entries += (f.rap_entry > 0.99) as usize;
                mosh_hops += (f.mosh > 0.5) as usize;
                flat_sum += raw.flatness as f64;
                flats.push(raw.flatness);
                for (s, v) in parts_sum.iter_mut().zip(raw.mosh_parts) {
                    *s += v as f64;
                }
                if f.drop > last_drop + 0.2 {
                    drop_times.push(format!("{:.0}:{:02.0}", (t / 60.0).floor(), t % 60.0));
                }
                last_drop = f.drop;
            }
            let mut signals = Signals::from_features(f);
            signals.0[Signal::BeatKick as usize] = clock.step(f, dt);
            transforms.step(&tuning, &signals, dt);
            motion.step(&mut panes, Drive::from_tuning(&tuning, &transforms, &clock), spring, screen, dt, &mut rng);
            let glints = tuning.control(crate::glass::Control::Glints, &transforms);
            light.step(&panes, glints, crate::glass_light::Tiers::from_tuning(&tuning, &transforms), screen, dt, &mut rng);
            if clock.one() != last_one {
                last_one = clock.one();
                one_moves += 1;
            }
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
                // The lead pitch on the spectrum's axis (30 Hz … 12 kHz, log); 0 if none.
                if raw.melody_hz > 0.0 { (raw.melody_hz / 30.0).ln() / 400f32.ln() } else { 0.0 },
            ];
            values.extend(signals.0);
            values.extend(transforms.0);
            values.extend(controls.iter().map(|&c| {
                let (lo, hi) = view_range(c);
                (tuning.control_value(c, &transforms) - lo) / (hi - lo)
            }));
            // Mean pane offset, as a share of the most allowed.
            let mean = panes.seeds.iter().zip(&panes.homes).map(|(s, h)| s.distance(*h)).sum::<f32>()
                / panes.seeds.len() as f32
                / panes.spacing;
            offsets.push(mean);
            values.push(mean / MAX_OFFSET);
            for (k, &v) in values.iter().enumerate() {
                lanes[k].push(q(v));
                sums[k] += v as f64;
                saturated[k] += (v > 0.9) as usize;
            }
            flux_raw.push(raw.flux);
            tempos.push(raw.tempo_bpm);
            spec_lead_db.extend_from_slice(&raw.lead_db);
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
    let duration = frames_total as f32 / rate as f32;
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
    // The lead spectrum, 50 dB below the track's 99.5th percentile up to it.
    let mut sorted_lead = spec_lead_db.clone();
    sorted_lead.sort_by(f32::total_cmp);
    let top = sorted_lead.get(sorted_lead.len() * 995 / 1000).copied().unwrap_or(0.0);
    let spec_lead: Vec<u8> = spec_lead_db.iter().map(|&d| q(db01(d, top - 50.0, top))).collect();
    let _ = writeln!(js, "  leadRows: {},\n  specLead: {:?},", crate::analysis::LEAD_ROWS, b64(&spec_lead));
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
    offsets.sort_by(f32::total_cmp);
    let pc = |p: usize| offsets.get(offsets.len() * p / 100).copied().unwrap_or(0.0);
    println!(
        "  glass panes: mean offset (x spacing) median {:.3}, 90th pct {:.3}, max {:.3} (limit {MAX_OFFSET}); ripples {:.0}/min",
        pc(50),
        pc(90),
        offsets.last().copied().unwrap_or(0.0),
        motion.started as f32 / mins
    );
    println!("  bar: the 1 moved {one_moves} times ({:.1}/min)", one_moves as f32 / mins);
    println!("  glass light: {:.0} glints/min", light.flared as f32 / mins);
    println!(
        "  lead line: a pitch {:.0}% of the time, notes {:.0} ms on average",
        100.0 * voiced_hops as f32 / hop.max(1) as f32,
        1000.0 * note_hops as f32 * dt / notes.max(1) as f32
    );
    println!(
        "  voice: singing {:.0}% of the time, rapping {:.0}%, rap entries {rap_entries}",
        100.0 * voice_hops as f32 / heard_hops.max(1) as f32,
        100.0 * rap_hops as f32 / heard_hops.max(1) as f32
    );
    flats.sort_by(f32::total_cmp);
    let fp = |p: usize| flats.get(flats.len() * p / 100).copied().unwrap_or(0.0);
    println!("  flatness p10 {:.3} p50 {:.3} p90 {:.3}", fp(10), fp(50), fp(90));
    let h = heard_hops.max(1) as f64;
    println!("  mosh parts (mean): loud {:.2} beat {:.2} heavy {:.2}", parts_sum[0] / h, parts_sum[1] / h, parts_sum[2] / h);
    println!(
        "  energy: mosh pit {:.0}% of the time, flatness {:.3}, drops at {}",
        100.0 * mosh_hops as f32 / heard_hops.max(1) as f32,
        flat_sum / heard_hops.max(1) as f64,
        if drop_times.is_empty() { "-".to_string() } else { drop_times.join(" ") }
    );
    line("signals", first_signal..first_transform, Signal::ALL.map(|s| s.key()).to_vec());
    line("transforms", first_transform..first_control, Transformation::ALL.map(|t| t.spec().key).to_vec());
    let mut first = first_control;
    for scene in Scene::ALL {
        let n = scene.controls().count();
        line(&format!("{} controls", scene.label()), first..first + n, scene.controls().map(|c| c.spec().key).collect());
        first += n;
    }
    println!(
        "  strongest vortex (time share): large {:.0}%  medium {:.0}%  small {:.0}%  tiny {:.0}%",
        pct(vortex_wins[0]),
        pct(vortex_wins[1]),
        pct(vortex_wins[2]),
        pct(vortex_wins[3])
    );
    Ok(name)
}


#[cfg(test)]
mod probe {
    /// Run by hand: `PROBE=file FROM=s TO=s cargo test -- --ignored energy_probe --nocapture`
    /// prints what the drop and mosh detectors see, once a second.
    #[test]
    #[ignore]
    fn energy_probe() {
        let path = std::env::var("PROBE").unwrap_or("w-fernandez.mp4".into());
        let from: f32 = std::env::var("FROM").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0);
        let to: f32 = std::env::var("TO").ok().and_then(|v| v.parse().ok()).unwrap_or(60.0);
        let (samples, ch, rate) = crate::audio::decode_file(std::path::Path::new(&path)).unwrap();
        let stereo: Vec<f32> = samples.chunks(ch).flat_map(|c| [c[0], c[if ch > 1 { 1 } else { 0 }]]).collect();
        let mut a = crate::analysis::Analyzer::new(rate as f32);
        let dt = crate::analysis::HOP as f32 / rate as f32;
        let mut hop = 0usize;
        for chunk in stereo.chunks(2 * crate::analysis::HOP) {
            a.push_stereo(chunk, |f, raw| {
                let t = hop as f32 * dt;
                hop += 1;
                if t >= from && t <= to && hop % (1.0 / dt) as usize == 0 {
                    println!(
                        "{:2.0}:{:04.1} drive {:.2}/{:.2} loud {:.2} bass {:.2} density {:.2} pulse {:.2} flat {:.3} mosh {:.2} drop {:.2}",
                        (t / 60.0).floor(), t % 60.0, raw.drive[0], raw.drive[1], f.loudness, f.bass, f.note_density, f.pulse_clarity, raw.flatness, f.mosh, f.drop
                    );
                }
            });
        }
    }
}
