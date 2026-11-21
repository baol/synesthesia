use bevy::prelude::*;

use crate::{
    bubble_controls::P,
    scene::{Scene, SceneParam},
    tuning::Knob,
};

/// Start-up settings from the command line. Everything the music steers, and
/// every other tunable, lives in [`crate::tuning::Tuning`] instead; flags such
/// as `--bubbles` only override it for this run.
#[derive(Resource, Debug, Clone)]
pub struct Config {
    pub width: u32,
    pub height: u32,
    pub novsync: bool,
    /// Start in borderless fullscreen (`F` or Alt+Enter toggles it).
    pub fullscreen: bool,
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
    /// Isolate one control (see `solo.rs`).
    pub solo: Option<String>,
    /// `--scene NAME`: start in this scene instead of the last one chosen.
    pub scene: Option<Scene>,
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
    /// `--scenarios`: write the synthetic test music (`scenarios.rs`) to
    /// `scenarios/`, analyse it into the timeline, and quit.
    pub scenarios: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            novsync: false,
            fullscreen: false,
            bench: None,
            screenshot: None,
            debug: false,
            prefs_open: false,
            no_prefs: false,
            spectrum: false,
            solo: None,
            scene: None,
            synth_speed: 1.0,
            synth_offset: 0.0,
            fixed_fps: None,
            scale_factor: None,
            live: Some(crate::audio::LiveSpec::Mic { device: None }),
            capture: None,
            capture_every: 0.5,
            overrides: Vec::new(),
            analyze: Vec::new(),
            scenarios: false,
        }
    }
}

/// The `--help` page.
fn usage() -> String {
    let scenes: Vec<_> = Scene::ALL.iter().map(|s| s.key()).collect();
    format!(
        "synestesia: music-driven visuals

USAGE: synestesia [OPTIONS]

Audio source (default: the microphone):
  --file PATH            play a file through the speakers and react to it
  --mute                 with --file: react without playing it
  --system               react to the computer's audio output
  --device NAME          use this input (part of its name)
  --list-devices         list the inputs and quit
  --synthetic            a built-in 120 BPM test track instead of live audio

Display:
  --scene NAME           start in this scene: {scenes}
  --fullscreen           start in borderless fullscreen
  --width N, --height N  window size (default 1920 x 1080)
  --spectrum             show the histogram
  --prefs                open the preferences panel
  --debug                show the debug overlay
  --no-prefs             ignore the saved preferences (and don't save)
  --bubbles N            number of bubbles
  --radius PX            bubble radius
  --plate PULL           pull of the plate's nodal lines (0 turns it off)
  --panes N              number of panes in the glass window
  --ground NAME          the ballroom's ground: {grounds}

Checking and recording:
  --solo NAME            one control on its own (--solo list shows them)
  --bench SECONDS        run that long and print the frame rate
  --screenshot PATH      with --bench: save a screenshot at the end
  --capture DIR          with --bench: save screenshots into DIR
  --capture-every S      seconds between captures (default 0.5)
  --fixed-fps N          advance time by exactly 1/N s per frame
  --novsync              don't wait for vertical sync
  --scale-factor F       override the display scale factor
  --synth-speed FACTOR   speed of the synthetic track's slow changes
  --analyze FILE         analyse a track offline and quit (repeatable)
  --scenarios            write and analyse the synthetic test tracks

  -h, --help             show this help

Keys: Q quit, F or Alt-Enter fullscreen, Tab next scene, Cmd-, / Ctrl-, preferences,
      H histogram, D debug overlay, M source (live, synthetic, silent).
",
        scenes = scenes.join(", "),
        grounds = crate::ballroom::GROUNDS.join(", ")
    )
}

/// A command-line mistake: say what, show the help, and quit with an error.
fn fail(message: &str) -> ! {
    eprintln!("synestesia: {message}\n");
    eprint!("{}", usage());
    std::process::exit(2);
}

/// An option's value as a number, or a command-line mistake.
fn number<T: std::str::FromStr>(option: &str, what: &str, value: &str) -> T {
    value.parse().unwrap_or_else(|_| fail(&format!("{option} needs {what}, got \"{value}\"")))
}

impl Config {
    pub fn from_args() -> Self {
        let mut cfg = Self::default();
        let mut args = std::env::args().skip(1);
        // `--mute` may come before `--file`.
        let mut mute_next = false;
        while let Some(arg) = args.next() {
            let mut val = || args.next().unwrap_or_else(|| fail(&format!("{arg} needs a value")));
            let mut num = |what: &str| -> f32 { number(&arg, what, &val()) };
            match arg.as_str() {
                "-h" | "--help" => {
                    print!("{}", usage());
                    std::process::exit(0);
                }
                "--bubbles" => cfg.overrides.push((Knob::Param(P::Bubbles.id()), num("a number of bubbles"))),
                "--ground" => {
                    let name = val();
                    let i = crate::ballroom::GROUNDS.iter().position(|g| *g == name).unwrap_or_else(|| fail(&format!("--ground needs one of {}, not \"{name}\"", crate::ballroom::GROUNDS.join(", "))));
                    cfg.overrides.push((Knob::Param(crate::ballroom::P::Ground.id()), i as f32));
                }
                "--panes" => cfg.overrides.push((Knob::Param(crate::glass::P::Panes.id()), num("a number of panes"))),
                "--radius" => cfg.overrides.push((Knob::Param(P::Radius.id()), num("a radius in pixels"))),
                "--plate" => cfg.overrides.push((Knob::Param(P::PlatePull.id()), num("a pull (0 turns it off)"))),
                "--width" => cfg.width = number(&arg, "a width in pixels", &val()),
                "--height" => cfg.height = number(&arg, "a height in pixels", &val()),
                "--bench" => cfg.bench = Some(number(&arg, "a number of seconds", &val())),
                "--screenshot" => cfg.screenshot = Some(val()),
                "--novsync" => cfg.novsync = true,
                "--fullscreen" => cfg.fullscreen = true,
                "--debug" => cfg.debug = true,
                "--prefs" => cfg.prefs_open = true,
                "--no-prefs" => cfg.no_prefs = true,
                "--spectrum" => cfg.spectrum = true,
                "--analyze" => cfg.analyze.push(val()),
                "--scenarios" => cfg.scenarios = true,
                "--scene" => {
                    let name = val();
                    let Some(scene) = Scene::find(&name) else {
                        let names: Vec<_> = Scene::ALL.iter().map(|s| s.key()).collect();
                        fail(&format!("unknown scene \"{name}\"; choose one of: {}", names.join(", ")));
                    };
                    cfg.scene = Some(scene);
                }
                "--solo" => {
                    let name = val();
                    if name == "json" {
                        crate::solo::print_manifest();
                        std::process::exit(0);
                    }
                    if crate::solo::find(&name).is_none() {
                        if name != "list" {
                            eprintln!("unknown solo {name}; choose one of:");
                        }
                        crate::solo::print_list();
                        std::process::exit(if name == "list" { 0 } else { 2 });
                    }
                    cfg.solo = Some(name);
                }
                "--synth-speed" => cfg.synth_speed = number(&arg, "a speed factor", &val()),
                "--fixed-fps" => cfg.fixed_fps = Some(number(&arg, "a frame rate", &val())),
                "--scale-factor" => cfg.scale_factor = Some(number(&arg, "a scale factor", &val())),
                "--synthetic" => cfg.live = None,
                "--system" => cfg.live = Some(crate::audio::LiveSpec::System),
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
                "--capture-every" => cfg.capture_every = number(&arg, "a number of seconds", &val()),
                other => fail(&format!("unknown option {other}")),
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
        if let Some(scene) = self.scene {
            t.scene = scene;
        }
        if let Some(solo) = self.solo.as_deref().and_then(crate::solo::find) {
            crate::solo::isolate(&mut t, solo);
        }
        t
    }
}
