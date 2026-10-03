//! The composition: a grid of layers × columns (scenes), the A/B crossfader, master effects
//! and launch quantization.

use crate::clip::{Clip, next_id};
use crate::effects::Effect;
use crate::modulation::Clock;
use crate::param::{Param, Params, Spec};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Blend {
    Normal,
    Add,
    Screen,
    Multiply,
    Difference,
    Lighten,
    Darken,
    Overlay,
    Subtract,
}

impl Blend {
    pub const ALL: [Blend; 9] = [
        Blend::Normal,
        Blend::Add,
        Blend::Screen,
        Blend::Multiply,
        Blend::Difference,
        Blend::Lighten,
        Blend::Darken,
        Blend::Overlay,
        Blend::Subtract,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Blend::Normal => "Normal",
            Blend::Add => "Add",
            Blend::Screen => "Screen",
            Blend::Multiply => "Multiply",
            Blend::Difference => "Difference",
            Blend::Lighten => "Lighten",
            Blend::Darken => "Darken",
            Blend::Overlay => "Overlay",
            Blend::Subtract => "Subtract",
        }
    }
}

/// Crossfader assignment.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Both,
    A,
    B,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Quantize {
    Off,
    Beat,
    Bar,
}

impl Quantize {
    pub const ALL: [Quantize; 3] = [Quantize::Off, Quantize::Beat, Quantize::Bar];
    pub fn name(self) -> &'static str {
        match self {
            Quantize::Off => "Launch: now",
            Quantize::Beat => "Launch: next beat",
            Quantize::Bar => "Launch: next bar",
        }
    }
    fn beats(self) -> f64 {
        match self {
            Quantize::Off => 0.0,
            Quantize::Beat => 1.0,
            Quantize::Bar => 4.0,
        }
    }
}

pub struct Layer {
    pub id: u64,
    pub name: String,
    pub clips: Vec<Option<Clip>>,
    /// Playing column.
    pub active: Option<usize>,
    /// Column fading out during a transition, and the 0..1 progress.
    pub fade_from: Option<usize>,
    pub fade: f32,
    pub opacity: Param,
    pub transition: Param,
    pub pos_x: Param,
    pub pos_y: Param,
    pub scale: Param,
    pub rotation: Param,
    pub blend: Blend,
    pub side: Side,
    pub bypass: bool,
    pub solo: bool,
    pub effects: Vec<Effect>,
}

impl Layer {
    pub fn new(name: String, columns: usize) -> Self {
        Self {
            id: next_id(),
            name,
            clips: (0..columns).map(|_| None).collect(),
            active: None,
            fade_from: None,
            fade: 1.0,
            opacity: Param::new(Spec::new("opacity", 0.0, 1.0, 1.0)),
            transition: Param::new(Spec::new("transition (s)", 0.0, 4.0, 0.0)),
            pos_x: Param::new(Spec::new("position x", -1.0, 1.0, 0.0)),
            pos_y: Param::new(Spec::new("position y", -1.0, 1.0, 0.0)),
            scale: Param::new(Spec::new("scale", 0.05, 4.0, 1.0).log()),
            rotation: Param::new(Spec::new("rotation °", -180.0, 180.0, 0.0)),
            blend: Blend::Normal,
            side: Side::Both,
            bypass: false,
            solo: false,
            effects: Vec::new(),
        }
    }

    pub fn active_clip(&self) -> Option<&Clip> {
        self.active.and_then(|c| self.clips.get(c)?.as_ref())
    }

    /// Start the clip in `col` (restarting it if it's already playing). An empty cell clears
    /// the layer, like launching an Ableton scene with an empty slot.
    pub fn launch(&mut self, col: usize) {
        if self.clips.get(col).is_none_or(|c| c.is_none()) {
            self.clear();
            return;
        }
        if self.active != Some(col) && self.transition.value > 0.0 && self.active_clip().is_some_and(|c| c.visible()) {
            self.fade_from = self.active;
            self.fade = 0.0;
        } else {
            self.fade_from = None;
            self.fade = 1.0;
        }
        self.active = Some(col);
        if let Some(c) = self.clips[col].as_mut() {
            c.restart();
        }
    }

    pub fn clear(&mut self) {
        self.active = None;
        self.fade_from = None;
        self.fade = 1.0;
    }

    pub fn remove_clip(&mut self, col: usize) {
        if let Some(cell) = self.clips.get_mut(col) {
            *cell = None;
        }
        if self.active == Some(col) {
            self.clear();
        }
        if self.fade_from == Some(col) {
            self.fade_from = None;
        }
    }

    /// Clips to draw, bottom first, with their opacity (handles transitions).
    pub fn draw_list(&self) -> Vec<(usize, f32)> {
        let mut out = Vec::new();
        if let Some(from) = self.fade_from
            && self.clips.get(from).is_some_and(|c| c.as_ref().is_some_and(|c| c.visible()))
        {
            out.push((from, 1.0));
        }
        if let Some(a) = self.active
            && self.clips.get(a).is_some_and(|c| c.as_ref().is_some_and(|c| c.visible()))
        {
            out.push((a, if out.is_empty() { 1.0 } else { self.fade }));
        }
        out
    }

    fn tick(&mut self, dt: f64, clock: Clock, playing: bool) {
        for p in [
            &mut self.opacity,
            &mut self.transition,
            &mut self.pos_x,
            &mut self.pos_y,
            &mut self.scale,
            &mut self.rotation,
        ] {
            p.tick(clock);
        }
        for e in &mut self.effects {
            e.visit_params(&mut |_, p| p.tick(clock));
        }
        if self.fade_from.is_some() {
            let secs = self.transition.get().max(0.001) as f64;
            self.fade = (self.fade + (dt / secs) as f32).min(1.0);
            if self.fade >= 1.0 {
                self.fade_from = None;
            }
        }
        for col in [self.active, self.fade_from].into_iter().flatten() {
            if let Some(Some(c)) = self.clips.get_mut(col) {
                c.tick(dt, clock, playing);
            }
        }
    }
}

impl Params for Layer {
    fn visit_params(&mut self, f: &mut dyn FnMut(&str, &mut Param)) {
        f("opacity", &mut self.opacity);
        f("transition", &mut self.transition);
        f("position x", &mut self.pos_x);
        f("position y", &mut self.pos_y);
        f("scale", &mut self.scale);
        f("rotation", &mut self.rotation);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Launch {
    Clip { layer: usize, col: usize },
    Column(usize),
}

pub struct Composition {
    /// Bottom layer first (drawn first).
    pub layers: Vec<Layer>,
    pub columns: usize,
    pub master: Param,
    pub crossfader: Param,
    pub effects: Vec<Effect>,
    pub bpm: f32,
    pub quantize: Quantize,
    pub pending: Vec<Launch>,
    pub playing: bool,
    /// Column most recently launched as a scene.
    pub active_column: Option<usize>,
}

impl Composition {
    pub fn new(layers: usize, columns: usize) -> Self {
        Self {
            layers: (0..layers).map(|i| Layer::new(format!("Layer {}", i + 1), columns)).collect(),
            columns,
            master: Param::new(Spec::new("master", 0.0, 1.0, 1.0)),
            crossfader: Param::new(Spec::new("crossfader", 0.0, 1.0, 0.5)),
            effects: Vec::new(),
            bpm: 120.0,
            quantize: Quantize::Off,
            pending: Vec::new(),
            playing: true,
            active_column: None,
        }
    }

    pub fn add_layer(&mut self) {
        let n = self.layers.len() + 1;
        self.layers.push(Layer::new(format!("Layer {n}"), self.columns));
    }

    pub fn add_column(&mut self) {
        self.columns += 1;
        for l in &mut self.layers {
            l.clips.push(None);
        }
    }

    pub fn ensure_columns(&mut self, n: usize) {
        while self.columns < n {
            self.add_column();
        }
    }

    pub fn clip_mut(&mut self, layer: usize, col: usize) -> Option<&mut Clip> {
        self.layers.get_mut(layer)?.clips.get_mut(col)?.as_mut()
    }

    pub fn set_clip(&mut self, layer: usize, col: usize, clip: Clip) {
        self.ensure_columns(col + 1);
        if let Some(l) = self.layers.get_mut(layer) {
            if l.active == Some(col) {
                l.clear();
            }
            l.clips[col] = Some(clip);
        }
    }

    /// Queue (or, without quantization, perform) a launch.
    pub fn launch(&mut self, what: Launch) {
        if self.quantize == Quantize::Off {
            self.perform(what);
        } else {
            // A newer request for the same layer/scene replaces the pending one.
            self.pending.retain(|p| !same_target(*p, what));
            self.pending.push(what);
        }
    }

    fn perform(&mut self, what: Launch) {
        match what {
            Launch::Clip { layer, col } => {
                if let Some(l) = self.layers.get_mut(layer) {
                    l.launch(col);
                }
            }
            Launch::Column(col) => {
                for l in &mut self.layers {
                    l.launch(col);
                }
                self.active_column = Some(col);
            }
        }
    }

    pub fn is_pending(&self, what: Launch) -> bool {
        self.pending.contains(&what)
    }

    /// Crossfader gain for a layer.
    pub fn side_gain(&self, side: Side) -> f32 {
        let x = self.crossfader.get();
        match side {
            Side::Both => 1.0,
            Side::A => (2.0 * (1.0 - x)).min(1.0),
            Side::B => (2.0 * x).min(1.0),
        }
    }

    /// Is the layer drawn at all (bypass / solo)?
    pub fn layer_audible(&self, i: usize) -> bool {
        let any_solo = self.layers.iter().any(|l| l.solo);
        let l = &self.layers[i];
        !l.bypass && (!any_solo || l.solo)
    }

    /// Advance one tick. `prev_beat` is the beat count before this tick.
    pub fn tick(&mut self, dt: f64, prev_beat: f64, clock: Clock) {
        let q = self.quantize.beats();
        if !self.pending.is_empty() && (q == 0.0 || (clock.beat / q).floor() > (prev_beat / q).floor()) {
            for what in std::mem::take(&mut self.pending) {
                self.perform(what);
            }
        }
        self.master.tick(clock);
        self.crossfader.tick(clock);
        for e in &mut self.effects {
            e.visit_params(&mut |_, p| p.tick(clock));
        }
        let playing = self.playing;
        for l in &mut self.layers {
            l.tick(dt, clock, playing);
        }
    }

    /// Every parameter with a readable path, for the automation overview.
    pub fn visit_all(&mut self, f: &mut dyn FnMut(&str, &mut Param)) {
        f("Master › master", &mut self.master);
        f("Master › crossfader", &mut self.crossfader);
        for e in &mut self.effects {
            let name = e.def().name;
            e.visit_params(&mut |label, p| f(&format!("Master › {name} › {label}"), p));
        }
        for l in &mut self.layers {
            let lname = l.name.clone();
            l.visit_params(&mut |label, p| f(&format!("{lname} › {label}"), p));
            for e in &mut l.effects {
                let name = e.def().name;
                e.visit_params(&mut |label, p| f(&format!("{lname} › {name} › {label}"), p));
            }
            for c in l.clips.iter_mut().flatten() {
                let cname = c.name.clone();
                c.visit_params(&mut |label, p| f(&format!("{lname} › {cname} › {label}"), p));
            }
        }
    }
}

fn same_target(a: Launch, b: Launch) -> bool {
    match (a, b) {
        (Launch::Clip { layer: x, .. }, Launch::Clip { layer: y, .. }) => x == y,
        (Launch::Column(_), Launch::Column(_)) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock(beat: f64) -> Clock {
        Clock {
            beat,
            time: beat / 2.0,
            bpm: 120.0,
        }
    }

    fn comp() -> Composition {
        let mut c = Composition::new(2, 3);
        c.set_clip(0, 0, Clip::generator(0));
        c.set_clip(0, 1, Clip::generator(1));
        c.set_clip(1, 1, Clip::generator(2));
        c
    }

    #[test]
    fn column_launch_fires_scene_and_clears_empty_cells() {
        let mut c = comp();
        c.launch(Launch::Clip { layer: 1, col: 1 });
        assert_eq!(c.layers[1].active, Some(1));
        c.launch(Launch::Column(0));
        assert_eq!(c.layers[0].active, Some(0));
        assert_eq!(c.layers[1].active, None, "empty slot in the scene stops the layer");
    }

    #[test]
    fn quantized_launch_waits_for_the_bar() {
        let mut c = comp();
        c.quantize = Quantize::Bar;
        c.launch(Launch::Column(1));
        c.tick(0.016, 0.0, clock(1.5));
        assert_eq!(c.layers[0].active, None);
        c.tick(0.016, 3.9, clock(4.01));
        assert_eq!(c.layers[0].active, Some(1));
        assert!(c.pending.is_empty());
    }

    #[test]
    fn transition_fades_between_clips() {
        let mut c = comp();
        c.layers[0].transition.set(1.0);
        c.launch(Launch::Clip { layer: 0, col: 0 });
        c.launch(Launch::Clip { layer: 0, col: 1 });
        assert_eq!(c.layers[0].draw_list(), vec![(0, 1.0), (1, 0.0)]);
        c.tick(0.5, 0.0, clock(1.0));
        let dl = c.layers[0].draw_list();
        assert_eq!(dl[1].0, 1);
        assert!((dl[1].1 - 0.5).abs() < 1e-4);
        c.tick(0.6, 1.0, clock(2.2));
        assert_eq!(c.layers[0].draw_list(), vec![(1, 1.0)]);
    }

    #[test]
    fn crossfader_and_solo() {
        let mut c = comp();
        c.crossfader.set(0.0);
        assert_eq!(c.side_gain(Side::A), 1.0);
        assert_eq!(c.side_gain(Side::B), 0.0);
        c.layers[1].solo = true;
        assert!(!c.layer_audible(0));
        assert!(c.layer_audible(1));
    }
}
