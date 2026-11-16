use bevy::prelude::*;

use crate::tuning::{Knob, P};

/// Start-up settings from the command line. Everything the music steers, and
/// every other tunable, lives in [`crate::tuning::Tuning`] instead; flags such
/// as `--bubbles` only override it for this run.
#[derive(Resource, Debug, Clone)]
pub struct Config {
    pub width: u32,
    pub height: u32,
    pub novsync: bool,
    /// Quit after this many seconds and print the average FPS.
    pub bench: Option<f32>,
    /// At the end of `--bench`, save a screenshot here before quitting.
    pub screenshot: Option<String>,
    /// During `--bench`, save a screenshot into this folder every `capture_every` s.
    pub capture: Option<String>,
    pub capture_every: f32,
    /// Start with the debug overlay on.
    pub debug: bool,
    /// Start with the preferences panel open.
    pub prefs_open: bool,
    /// Ignore the saved preferences (and don't save over them).
    pub no_prefs: bool,
    /// Start with the spectrum histogram shown.
    pub spectrum: bool,
    /// Isolate one motion control (see `solo.rs`).
    pub solo: Option<String>,
    /// Synthetic track: speed of its slow changes (1 = normal), and start offset (s).
    pub synth_speed: f32,
    pub synth_offset: f32,
    /// Advance time by exactly 1/N s per frame (for recording clips).
    pub fixed_fps: Option<f32>,
    /// Override the display scale factor (1 = one pixel per logical pixel).
    pub scale_factor: Option<f32>,
    /// Live audio source; `None` uses the synthetic track.
    pub live: Option<crate::audio::LiveSpec>,
    /// Tuning values set on the command line.
    pub overrides: Vec<(Knob, f32)>,
    /// `--analyze FILE…`: analyse these tracks offline and quit.
    pub analyze: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            novsync: false,
            bench: None,
            screenshot: None,
            debug: false,
            prefs_open: false,
            no_prefs: false,
            spectrum: false,
            solo: None,
            synth_speed: 1.0,
            synth_offset: 0.0,
            fixed_fps: None,
            scale_factor: None,
            live: Some(crate::audio::LiveSpec::Mic { device: None }),
            capture: None,
            capture_every: 0.5,
            overrides: Vec::new(),
            analyze: Vec::new(),
        }
    }
}

impl Config {
    pub fn from_args() -> Self {
        let mut cfg = Self::default();
        let mut args = std::env::args().skip(1);
        // `--mute` may come before `--file`.
        let mut mute_next = false;
        while let Some(arg) = args.next() {
            let mut val = || args.next().unwrap_or_else(|| panic!("{arg} needs a value"));
            let mut num = |what: &str| -> f32 { val().parse().unwrap_or_else(|_| panic!("{what}")) };
            match arg.as_str() {
                "--bubbles" => cfg.overrides.push((Knob::Param(P::Bubbles), num("--bubbles N"))),
                "--radius" => cfg.overrides.push((Knob::Param(P::Radius), num("--radius PX"))),
                "--plate" => cfg.overrides.push((Knob::Param(P::PlatePull), num("--plate PULL (0 turns it off)"))),
                "--width" => cfg.width = val().parse().expect("--width PX"),
                "--height" => cfg.height = val().parse().expect("--height PX"),
                "--bench" => cfg.bench = Some(val().parse().expect("--bench SECONDS")),
                "--screenshot" => cfg.screenshot = Some(val()),
                "--novsync" => cfg.novsync = true,
                "--debug" => cfg.debug = true,
                "--prefs" => cfg.prefs_open = true,
                "--no-prefs" => cfg.no_prefs = true,
                "--spectrum" => cfg.spectrum = true,
                "--analyze" => cfg.analyze.push(val()),
                "--solo" => {
                    let name = val();
                    if crate::solo::find(&name).is_none() {
                        if name != "list" {
                            eprintln!("unknown solo {name}; choose one of:");
                        }
                        crate::solo::print_list();
                        std::process::exit(if name == "list" { 0 } else { 2 });
                    }
                    cfg.solo = Some(name);
                }
                "--synth-speed" => cfg.synth_speed = val().parse().expect("--synth-speed FACTOR"),
                "--fixed-fps" => cfg.fixed_fps = Some(val().parse().expect("--fixed-fps N")),
                "--scale-factor" => cfg.scale_factor = Some(val().parse().expect("--scale-factor F")),
                "--synthetic" => cfg.live = None,
                "--device" => cfg.live = Some(crate::audio::LiveSpec::Mic { device: Some(val()) }),
                "--file" => {
                    let path = val().into();
                    let mute = matches!(cfg.live, Some(crate::audio::LiveSpec::File { mute: true, .. }));
                    cfg.live = Some(crate::audio::LiveSpec::File { path, mute });
                }
                "--mute" => match &mut cfg.live {
                    Some(crate::audio::LiveSpec::File { mute, .. }) => *mute = true,
                    _ => mute_next = true,
                },
                "--list-devices" => {
                    crate::audio::list_input_devices();
                    std::process::exit(0);
                }
                "--capture" => cfg.capture = Some(val()),
                "--capture-every" => cfg.capture_every = val().parse().expect("--capture-every SECONDS"),
                other => eprintln!("ignoring unknown argument {other}"),
            }
        }
        if mute_next && let Some(crate::audio::LiveSpec::File { mute, .. }) = &mut cfg.live {
            *mute = true;
        }
        if let Some(solo) = cfg.solo.as_deref().and_then(crate::solo::find) {
            // Solos are for review: repeatable synthetic music, saved prefs untouched.
            cfg.synth_offset = crate::solo::synth_offset(solo);
            cfg.no_prefs = true;
            cfg.live = None;
        }
        cfg
    }

    /// The tuning to start with: saved preferences (unless `--no-prefs`),
    /// then the command-line overrides.
    pub fn initial_tuning(&self) -> crate::tuning::Tuning {
        let mut t = if self.no_prefs { Default::default() } else { crate::tuning::Tuning::load() };
        for &(k, v) in &self.overrides {
            t.set_knob(k, v);
        }
        t.show_spectrum |= self.spectrum;
        if let Some(solo) = self.solo.as_deref().and_then(crate::solo::find) {
            crate::solo::isolate(&mut t, solo);
        }
        t
    }
}
