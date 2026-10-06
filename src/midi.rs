//! MIDI control: play the punch pads, turn sliders and launch scenes from hardware.
//!
//! * Notes 36–51 (the usual drum-pad layout) play the 16 punch pads, in the Q…K order.
//! * *MIDI learn* maps any note or CC to a parameter, a pad, a scene or the Shift control
//!   (hold it and hit a pad to latch). Learned mappings win over the default pad notes.
//! * Optionally, MIDI clock sets the tempo and keeps the beat in phase.
//!
//! Mappings belong to the controller, not to a set: they're saved in `midi.json` in the
//! config directory. Parameters are stored by their script path (`2/fx1/rotate °`).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

/// A channel message (channels are 0-based here, 1-based in the UI).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Msg {
    NoteOn { ch: u8, note: u8, vel: u8 },
    NoteOff { ch: u8, note: u8 },
    Cc { ch: u8, cc: u8, value: u8 },
    Clock,
    Start,
    Continue,
    Stop,
}

impl Msg {
    pub fn parse(bytes: &[u8]) -> Option<Msg> {
        let status = *bytes.first()?;
        let ch = status & 0x0f;
        let data = |i: usize| bytes.get(i).copied();
        Some(match status {
            0xf8 => Msg::Clock,
            0xfa => Msg::Start,
            0xfb => Msg::Continue,
            0xfc => Msg::Stop,
            _ => match status & 0xf0 {
                0x90 if data(2)? > 0 => Msg::NoteOn { ch, note: data(1)?, vel: data(2)? },
                // Note-on with velocity 0 is how many devices send note-off.
                0x80 | 0x90 => Msg::NoteOff { ch, note: data(1)? },
                0xb0 => Msg::Cc { ch, cc: data(1)?, value: data(2)? },
                _ => return None,
            },
        })
    }

    pub fn describe(&self) -> String {
        match *self {
            Msg::NoteOn { ch, note, vel } => format!("note {note} on · vel {vel} · ch {}", ch + 1),
            Msg::NoteOff { ch, note } => format!("note {note} off · ch {}", ch + 1),
            Msg::Cc { ch, cc, value } => format!("CC {cc} = {value} · ch {}", ch + 1),
            Msg::Clock => "clock".into(),
            Msg::Start => "start".into(),
            Msg::Continue => "continue".into(),
            Msg::Stop => "stop".into(),
        }
    }
}

/// A physical control.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Control {
    Note { ch: u8, note: u8 },
    Cc { ch: u8, cc: u8 },
}

impl Control {
    pub fn describe(&self) -> String {
        match *self {
            Control::Note { ch, note } => format!("note {note} · ch {}", ch + 1),
            Control::Cc { ch, cc } => format!("CC {cc} · ch {}", ch + 1),
        }
    }
}

/// What a control drives.
#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub enum Target {
    /// A parameter, by script path.
    Param(String),
    Pad(usize),
    /// A scene (column), 0-based.
    Scene(usize),
    /// Hold to make pad notes latch instead of hold.
    Shift,
}

impl Target {
    pub fn describe(&self) -> String {
        match self {
            Target::Param(p) => p.clone(),
            Target::Pad(i) => format!("pad {}", crate::punch::PUNCHES[*i].name),
            Target::Scene(c) => format!("scene {}", c + 1),
            Target::Shift => "Shift".into(),
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Mapping {
    pub control: Control,
    pub target: Target,
}

/// What a message asks the app to do.
#[derive(Clone, PartialEq, Debug)]
pub enum Action {
    /// Set a parameter to a 0..1 position.
    Param(String, f32),
    Pad(usize, bool),
    ToggleLatch(usize),
    Launch(usize),
    /// From MIDI clock (when followed).
    Bpm(f32),
    /// A quarter note boundary of the incoming clock.
    Beat,
    /// The clock (re)started: the next beat is a downbeat.
    Start,
    /// A mapping was learned (and saved).
    Learned(Mapping),
}

/// First note of the default pad layout (C1 in most drum-pad mappings).
pub const PAD_BASE_NOTE: u8 = 36;

#[derive(Clone, PartialEq, Debug)]
pub enum Port {
    Off,
    All,
    Named(String),
}

pub struct Midi {
    pub port: Port,
    /// The port choice to save: only changed by `connect` (the user's choice), so scripts that
    /// switch the hardware off don't save that.
    saved_port: Port,
    pub mappings: Vec<Mapping>,
    /// Waiting for a control to map to this.
    pub learning: Option<Target>,
    pub follow_clock: bool,
    /// The last message, for the UI.
    pub last: Option<String>,
    shift: bool,
    clock: ClockFollower,
    /// Where mappings are saved; `None` keeps them in memory (scripts, tests).
    config: Option<PathBuf>,
    #[cfg(not(target_arch = "wasm32"))]
    connections: Vec<midir::MidiInputConnection<()>>,
    /// The browser build has no MIDI input, so never any connections.
    #[cfg(target_arch = "wasm32")]
    connections: Vec<()>,
    tx: Sender<(u64, Msg)>,
    rx: Receiver<(u64, Msg)>,
    pub error: Option<String>,
}

/// Names of the MIDI input ports. None in the browser build.
#[cfg(target_arch = "wasm32")]
pub fn ports() -> Vec<String> {
    Vec::new()
}

/// Names of the MIDI input ports.
#[cfg(not(target_arch = "wasm32"))]
pub fn ports() -> Vec<String> {
    let Ok(input) = midir::MidiInput::new("tripslop") else { return Vec::new() };
    input.ports().iter().filter_map(|p| input.port_name(p).ok()).collect()
}

/// Where tripslop keeps its settings: `TRIPSLOP_CONFIG_DIR`, else the platform's config dir.
pub fn config_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("TRIPSLOP_CONFIG_DIR") {
        return Some(PathBuf::from(d));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Application Support/tripslop"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("tripslop"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home.map(|h| h.join(".config"))).map(|d| d.join("tripslop"))
    }
}

impl Midi {
    /// `config`: the directory to load `midi.json` from and save it to (`None`: memory only).
    pub fn new(config: Option<PathBuf>) -> Self {
        let (tx, rx) = channel();
        let mut m = Self {
            port: Port::All,
            saved_port: Port::All,
            mappings: Vec::new(),
            learning: None,
            follow_clock: false,
            last: None,
            shift: false,
            clock: ClockFollower::default(),
            config: config.map(|d| d.join("midi.json")),
            connections: Vec::new(),
            tx,
            rx,
            error: None,
        };
        if let Some(path) = m.config.clone() {
            m.load(&path);
        }
        m
    }

    /// Connect to the chosen port(s).
    pub fn connect(&mut self, port: Port) {
        self.connections.clear();
        self.error = None;
        self.port = port.clone();
        self.saved_port = port;
        if self.port != Port::Off {
            let wanted: Vec<String> = ports().into_iter().filter(|n| self.port == Port::All || self.port == Port::Named(n.clone())).collect();
            for name in &wanted {
                if let Err(e) = self.open(name) {
                    self.error = Some(format!("{name}: {e}"));
                }
            }
        }
        self.save();
    }

    #[cfg(target_arch = "wasm32")]
    fn open(&mut self, _name: &str) -> Result<(), String> {
        Err("MIDI isn't available in the browser build".into())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn open(&mut self, name: &str) -> Result<(), String> {
        let mut input = midir::MidiInput::new("tripslop").map_err(|e| e.to_string())?;
        // Keep clock messages; drop sysex and active sensing.
        input.ignore(midir::Ignore::SysexAndActiveSense);
        let port = input.ports().into_iter().find(|p| input.port_name(p).is_ok_and(|n| n == name)).ok_or("port went away")?;
        let tx = self.tx.clone();
        let conn = input
            .connect(
                &port,
                "tripslop-in",
                move |stamp, bytes, _| {
                    if let Some(m) = Msg::parse(bytes) {
                        let _ = tx.send((stamp, m));
                    }
                },
                (),
            )
            .map_err(|e| e.to_string())?;
        self.connections.push(conn);
        Ok(())
    }

    pub fn connected(&self) -> usize {
        self.connections.len()
    }

    /// Handle what arrived from the hardware since the last call.
    pub fn poll(&mut self) -> Vec<Action> {
        let mut actions = Vec::new();
        while let Ok((stamp, msg)) = self.rx.try_recv() {
            actions.extend(self.handle(msg, stamp));
        }
        actions
    }

    /// Handle one message. `stamp` is in microseconds (for clock timing).
    pub fn handle(&mut self, msg: Msg, stamp: u64) -> Vec<Action> {
        match msg {
            Msg::Clock | Msg::Start | Msg::Continue | Msg::Stop => {
                if !self.follow_clock {
                    return Vec::new();
                }
                return self.clock.handle(msg, stamp);
            }
            _ => self.last = Some(msg.describe()),
        }
        let (control, value, on) = match msg {
            Msg::NoteOn { ch, note, vel } => (Control::Note { ch, note }, vel as f32 / 127.0, true),
            Msg::NoteOff { ch, note } => (Control::Note { ch, note }, 0.0, false),
            Msg::Cc { ch, cc, value } => (Control::Cc { ch, cc }, value as f32 / 127.0, value >= 64),
            _ => unreachable!(),
        };
        if let Some(target) = self.learning.clone() {
            // A note-off (the release of the note just learned) doesn't count.
            if !matches!(msg, Msg::NoteOff { .. }) {
                self.learning = None;
                let m = Mapping { control, target };
                self.mappings.retain(|x| x.target != m.target && x.control != m.control);
                self.mappings.push(m.clone());
                self.save();
                return vec![Action::Learned(m)];
            }
            return Vec::new();
        }
        let target = match self.mappings.iter().find(|m| m.control == control) {
            Some(m) => m.target.clone(),
            None => match control {
                Control::Note { note, .. } if (PAD_BASE_NOTE..PAD_BASE_NOTE + 16).contains(&note) => Target::Pad((note - PAD_BASE_NOTE) as usize),
                _ => return Vec::new(),
            },
        };
        let is_note = matches!(control, Control::Note { .. });
        match target {
            // A note sweeps a parameter fully: on = top, off = bottom.
            Target::Param(path) => vec![Action::Param(path, if is_note { if on { 1.0 } else { 0.0 } } else { value })],
            Target::Pad(i) if self.shift => match on {
                true => vec![Action::ToggleLatch(i)],
                false => Vec::new(),
            },
            Target::Pad(i) => vec![Action::Pad(i, on)],
            Target::Scene(c) if on => vec![Action::Launch(c)],
            Target::Scene(_) => Vec::new(),
            Target::Shift => {
                self.shift = on;
                Vec::new()
            }
        }
    }

    pub fn forget(&mut self, target: &Target) {
        self.mappings.retain(|m| &m.target != target);
        self.save();
    }

    pub fn forget_all(&mut self) {
        self.mappings.clear();
        self.save();
    }

    pub fn set_follow_clock(&mut self, on: bool) {
        self.follow_clock = on;
        self.clock = ClockFollower::default();
        self.save();
    }

    fn load(&mut self, path: &Path) {
        let Ok(text) = std::fs::read_to_string(path) else { return };
        let Ok(doc) = serde_json::from_str::<serde_json::Value>(&text) else {
            self.error = Some(format!("{} isn't valid JSON; ignoring it", path.display()));
            return;
        };
        self.follow_clock = doc["follow_clock"].as_bool().unwrap_or(false);
        self.port = match doc["port"].as_str() {
            Some("off") => Port::Off,
            Some("all") | None => Port::All,
            Some(n) => Port::Named(n.to_string()),
        };
        self.saved_port = self.port.clone();
        self.mappings = doc["mappings"].as_array().into_iter().flatten().filter_map(mapping_from_json).collect();
    }

    fn save(&self) {
        let Some(path) = &self.config else { return };
        let port = match &self.saved_port {
            Port::Off => "off".to_string(),
            Port::All => "all".to_string(),
            Port::Named(n) => n.clone(),
        };
        let doc = serde_json::json!({
            "port": port,
            "follow_clock": self.follow_clock,
            "mappings": self.mappings.iter().map(mapping_to_json).collect::<Vec<_>>(),
        });
        let write = || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(path, serde_json::to_string_pretty(&doc).unwrap_or_default() + "\n")
        };
        if let Err(e) = write() {
            eprintln!("can't save {}: {e}", path.display());
        }
    }
}

fn mapping_to_json(m: &Mapping) -> serde_json::Value {
    let mut v = serde_json::Map::new();
    match m.control {
        Control::Note { ch, note } => {
            v.insert("note".into(), note.into());
            v.insert("channel".into(), (ch + 1).into());
        }
        Control::Cc { ch, cc } => {
            v.insert("cc".into(), cc.into());
            v.insert("channel".into(), (ch + 1).into());
        }
    }
    match &m.target {
        Target::Param(p) => v.insert("param".into(), p.clone().into()),
        Target::Pad(i) => v.insert("pad".into(), (i + 1).into()),
        Target::Scene(c) => v.insert("scene".into(), (c + 1).into()),
        Target::Shift => v.insert("shift".into(), true.into()),
    };
    v.into()
}

fn mapping_from_json(v: &serde_json::Value) -> Option<Mapping> {
    let ch = (v["channel"].as_u64()?.clamp(1, 16) - 1) as u8;
    let control = match (v["note"].as_u64(), v["cc"].as_u64()) {
        (Some(note), _) => Control::Note { ch, note: note.min(127) as u8 },
        (_, Some(cc)) => Control::Cc { ch, cc: cc.min(127) as u8 },
        _ => return None,
    };
    let one_based = |k: &str| v[k].as_u64().and_then(|n| n.checked_sub(1)).map(|n| n as usize);
    let target = if let Some(p) = v["param"].as_str() {
        Target::Param(p.to_string())
    } else if let Some(i) = one_based("pad").filter(|i| *i < 16) {
        Target::Pad(i)
    } else if let Some(c) = one_based("scene") {
        Target::Scene(c)
    } else if v["shift"].as_bool() == Some(true) {
        Target::Shift
    } else {
        return None;
    };
    Some(Mapping { control, target })
}

/// Tempo and phase from MIDI clock (24 pulses per quarter note).
#[derive(Default)]
struct ClockFollower {
    last: Option<u64>,
    /// Recent pulse intervals in microseconds.
    intervals: std::collections::VecDeque<f64>,
    pulses: u64,
}

impl ClockFollower {
    fn handle(&mut self, msg: Msg, stamp: u64) -> Vec<Action> {
        match msg {
            Msg::Start => {
                self.pulses = 0;
                self.last = None;
                vec![Action::Start]
            }
            Msg::Stop => {
                self.last = None;
                Vec::new()
            }
            Msg::Clock => {
                let mut out = Vec::new();
                if let Some(prev) = self.last
                    && stamp > prev
                {
                    self.intervals.push_back((stamp - prev) as f64);
                    if self.intervals.len() > 48 {
                        self.intervals.pop_front();
                    }
                    // A tempo once there's a beat's worth of pulses to average.
                    if self.intervals.len() >= 24 {
                        let avg = self.intervals.iter().sum::<f64>() / self.intervals.len() as f64;
                        let bpm = 60e6 / (avg * 24.0);
                        if (20.0..=400.0).contains(&bpm) {
                            out.push(Action::Bpm(bpm as f32));
                        }
                    }
                }
                self.last = Some(stamp);
                if self.pulses % 24 == 0 {
                    out.push(Action::Beat);
                }
                self.pulses += 1;
                out
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(note: u8, on: bool) -> Msg {
        if on { Msg::NoteOn { ch: 0, note, vel: 100 } } else { Msg::NoteOff { ch: 0, note } }
    }

    #[test]
    fn parses_channel_messages() {
        assert_eq!(Msg::parse(&[0x92, 36, 100]), Some(Msg::NoteOn { ch: 2, note: 36, vel: 100 }));
        assert_eq!(Msg::parse(&[0x90, 36, 0]), Some(Msg::NoteOff { ch: 0, note: 36 }));
        assert_eq!(Msg::parse(&[0x81, 36, 64]), Some(Msg::NoteOff { ch: 1, note: 36 }));
        assert_eq!(Msg::parse(&[0xb0, 1, 127]), Some(Msg::Cc { ch: 0, cc: 1, value: 127 }));
        assert_eq!(Msg::parse(&[0xf8]), Some(Msg::Clock));
        assert_eq!(Msg::parse(&[0xe0, 0, 64]), None); // pitch bend
    }

    #[test]
    fn default_pads_hold_and_shift_latches() {
        let mut m = Midi::new(None);
        assert_eq!(m.handle(note(36, true), 0), vec![Action::Pad(0, true)]);
        assert_eq!(m.handle(note(36, false), 0), vec![Action::Pad(0, false)]);
        assert_eq!(m.handle(note(51, true), 0), vec![Action::Pad(15, true)]);
        assert!(m.handle(note(60, true), 0).is_empty());
        // Learn a Shift button, then hold it while hitting a pad.
        m.learning = Some(Target::Shift);
        m.handle(Msg::Cc { ch: 0, cc: 64, value: 127 }, 0);
        m.handle(Msg::Cc { ch: 0, cc: 64, value: 127 }, 0);
        assert_eq!(m.handle(note(37, true), 0), vec![Action::ToggleLatch(1)]);
        assert!(m.handle(note(37, false), 0).is_empty());
    }

    #[test]
    fn learn_maps_and_replaces() {
        let mut m = Midi::new(None);
        m.learning = Some(Target::Param("2/fx1/rotate °".into()));
        let learned = m.handle(Msg::Cc { ch: 0, cc: 1, value: 10 }, 0);
        assert!(matches!(&learned[..], [Action::Learned(_)]));
        assert_eq!(m.handle(Msg::Cc { ch: 0, cc: 1, value: 127 }, 0), vec![Action::Param("2/fx1/rotate °".into(), 1.0)]);
        // The same knob learned for something else moves over; one target, one control.
        m.learning = Some(Target::Scene(2));
        m.handle(Msg::Cc { ch: 0, cc: 1, value: 0 }, 0);
        assert_eq!(m.mappings.len(), 1);
        assert_eq!(m.handle(Msg::Cc { ch: 0, cc: 1, value: 127 }, 0), vec![Action::Launch(2)]);
        // A learned note overrides the default pad layout.
        m.learning = Some(Target::Scene(0));
        m.handle(note(36, true), 0);
        assert_eq!(m.handle(note(36, true), 0), vec![Action::Launch(0)]);
    }

    #[test]
    fn mappings_survive_a_restart() {
        let dir = std::env::temp_dir().join(format!("tripslop-midi-{}", std::process::id()));
        let mut m = Midi::new(Some(dir.clone()));
        m.connect(Port::Off);
        m.learning = Some(Target::Param("1/opacity".into()));
        m.handle(Msg::Cc { ch: 3, cc: 7, value: 0 }, 0);
        m.learning = Some(Target::Pad(4));
        m.handle(Msg::NoteOn { ch: 9, note: 60, vel: 1 }, 0);
        let again = Midi::new(Some(dir.clone()));
        assert_eq!(again.mappings, m.mappings);
        assert_eq!(again.port, Port::Off);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn follows_clock_tempo_and_beats() {
        let mut m = Midi::new(None);
        assert!(m.handle(Msg::Clock, 0).is_empty(), "ignored unless following");
        m.follow_clock = true;
        assert_eq!(m.handle(Msg::Start, 0), vec![Action::Start]);
        // 128 BPM: a pulse every 60e6 / (128 * 24) µs.
        let step = 60e6 / (128.0 * 24.0);
        let mut bpm = 0.0;
        let mut beats = 0;
        for i in 0..96 {
            for a in m.handle(Msg::Clock, (i as f64 * step) as u64) {
                match a {
                    Action::Bpm(b) => bpm = b,
                    Action::Beat => beats += 1,
                    _ => {}
                }
            }
        }
        assert!((bpm - 128.0).abs() < 0.5, "{bpm}");
        assert_eq!(beats, 4);
    }
}
