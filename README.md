# synestesia

Synestesia listens to whatever is playing (the microphone, a
file, or the computer's own audio output) and makes it dance.

<table>
<tr>
<td width="50%"><img src="docs/bubbles.gif" alt="The bubbles scene: thousands of grey bubbles carried by wind and vortices, with the histogram at the bottom"></td>
<td width="50%"><img src="docs/glass.gif" alt="The glass window scene: a stained-glass window whose panes shake and dance, with the histogram at the bottom"></td>
</tr>
<tr>
<td><b>Bubbles</b>: shiny bubbles flow through wind, four vortices and turbulence, gather on the nodal lines of a vibrating plate, and hop on the beat.</td>
<td><b>Glass window</b>: a stained-glass window lit from behind. Its panes dance the salsa on the beat, ripple, breathe and shake when the band crashes in; a spotlight dances behind the glass while someone sings.</td>
</tr>
</table>

## Platforms

|                                   | macOS                       | Linux                          | Windows        |
|-----------------------------------|-----------------------------|--------------------------------|----------------|
| Runs                              | yes (Apple silicon)         | yes                            | not tested     |
| Microphone, any input (`--device`)| yes                         | yes                            | not tested     |
| A music file (`--file`)           | yes                         | yes                            | not tested     |
| System audio loopback (`--system`)| yes, macOS 14.6 or later    | yes, with PulseAudio, PipeWire | no             |
| Singing / voice detection         | yes (the system's sound classifier) | no (reads as no voice) | no             |

## Building

You need Rust 1.97.1 or later (Bevy 0.20's minimum).

```bash
cargo run --release
```

## Running

```bash
cargo run --release                               # listen to the microphone
cargo run --release -- --file song.flac           # play a file and react to it
cargo run --release -- --system                   # react to what the computer plays
cargo run --release -- --file song.mp3 --scene glass --fullscreen
```

### Main options

| Option | What it does |
|---|---|
| `--file PATH` | Play a file through the speakers and react to it |
| `--mute` | With `--file`: react without playing it |
| `--system` | React to the computer's audio output |
| `--device NAME` | Use this input (part of its name); `--list-devices` lists them |
| `--synthetic` | A built-in 120 BPM test track instead of live audio |
| `--scene bubbles\|glass` | Start in this scene (default: the last one shown) |
| `--fullscreen` | Start in borderless fullscreen |
| `--width N`, `--height N` | Window size (default 1920 × 1080) |
| `--spectrum` | Show the histogram |
| `--prefs` | Open the preferences panel |
| `--debug` | Show the debug overlay |
| `--no-prefs` | Ignore the saved preferences (and don't save) |
| `--bubbles N`, `--radius PX`, `--panes N` | Bubble count and size; panes in the window |

For checking and recording: `--solo CONTROL` (one control on its own; `--solo list`), `--bench S`
(run S seconds and print the frame rate), `--screenshot PATH`, `--capture DIR`,
`--fixed-fps N`, `--novsync`, `--scale-factor F`.

### Keys

| Key | |
|---|---|
| `Tab` | Next scene |
| `Cmd-,` (`Ctrl-,`) | Preferences: every transformation and control, live |
| `H` | Histogram |
| `D` | Debug overlay: every signal, transformation and control |
| `F`, `Alt-Enter` | Fullscreen |
| `M` | Source: live, synthetic, silent |
| `Space`, `B`, `1`–`4` | Fake an onset, a beat, a band (synthetic or silent source) |
| `Esc` | Close the preferences |

Preferences are saved when you let go of a slider, in
`~/Library/Application Support/synestesia/prefs.txt` on macOS and
`~/.config/synestesia/prefs.txt` on Linux.
