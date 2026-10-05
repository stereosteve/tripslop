//! Punch-in performance FX (after Teenage Engineering's OP-Z / KO II): hold one or more pads
//! for a momentary, beat-synced transformation, release and everything snaps back.
//!
//! Each pad is a small composite routine that runs every tick while it's active. It can
//! inject its own temporary effects into individual layers or the master chain, gate or pump
//! layers in beat-locked patterns, and take over clip playback (beat repeat, tape stop,
//! reverse). It works only through per-tick overrides (`LayerPerf`, `ClipPerf`,
//! `master_perf`) and its own effect instances, so the set itself is never modified.

use std::collections::HashMap;

use eframe::egui::Key;

use crate::clip::Roll;
use crate::composition::Composition;
use crate::effects::{Effect, EffectKind};
use crate::modulation::Clock;

pub struct PunchDef {
    pub name: &'static str,
    pub key: Key,
    pub hint: &'static str,
}

pub static PUNCHES: [PunchDef; 16] = [
    PunchDef { name: "Stutter", key: Key::Q, hint: "Beat repeat on every clip: 1/2 beat, then 1/4, then 1/8 the longer you hold" },
    PunchDef { name: "Chase", key: Key::W, hint: "Only one layer visible at a time, stepping through the layers on 1/16 notes" },
    PunchDef { name: "Mirror split", key: Key::E, hint: "Alternate layers mirror sideways / vertically; the top layer turns kaleidoscope" },
    PunchDef { name: "Pump", key: Key::R, hint: "Each layer zoom-pumps on the beat, phase-shifted per layer" },
    PunchDef { name: "Tape stop", key: Key::T, hint: "Clips slow to a halt over one beat while the picture sags and drains" },
    PunchDef { name: "Reverse", key: Key::Y, hint: "Clips play backwards with an RGB time split and a hue flip" },
    PunchDef { name: "Echo build", key: Key::U, hint: "1/16-note echo trails that thicken the longer you hold" },
    PunchDef { name: "Riser", key: Key::I, hint: "Two-bar build: zoom, blur and brightness climb; a strobe kicks in and accelerates" },
    PunchDef { name: "Strobe split", key: Key::A, hint: "Even layers on the beat, odd layers on the off-beat (1/8 notes)" },
    PunchDef { name: "Kaleido spin", key: Key::S, hint: "Master kaleidoscope spinning with the beat, segment count changing each beat" },
    PunchDef { name: "Tunnel", key: Key::D, hint: "Master feedback tunnel, turning the other way every bar" },
    PunchDef { name: "Fractal bloom", key: Key::F, hint: "The top layer blooms into a Sierpinski feedback fractal; the others dim" },
    PunchDef { name: "Glitch", key: Key::G, hint: "Random per-layer jumps on 1/16s, a random layer pixelates, CRT on the master" },
    PunchDef { name: "Invert flip", key: Key::H, hint: "Layers invert in alternation, flipping every beat" },
    PunchDef { name: "Wash out", key: Key::J, hint: "Whiteout transition: blur, brightness and desaturation build over a bar" },
    PunchDef { name: "Trance gate", key: Key::K, hint: "Master chopped by a 16-step gate pattern" },
];

const ATTACK_SECS: f32 = 0.03;
const RELEASE_SECS: f32 = 0.15;

#[derive(Default)]
pub struct Pad {
    pub key_held: bool,
    pub mouse_held: bool,
    /// Held by an automation script (`--script`).
    pub script_held: bool,
    /// Held by a MIDI note.
    pub midi_held: bool,
    pub latched: bool,
    /// Smoothed 0..1 envelope.
    pub amount: f32,
    /// Beat when the pad last became active.
    pub start_beat: f64,
    was_active: bool,
    /// Effect instances this pad injects, per layer id, and on the master.
    layer_fx: HashMap<u64, Vec<Effect>>,
    master_fx: Vec<Effect>,
    /// Beat-repeat anchors per clip id.
    rolls: HashMap<u64, f64>,
}

impl Pad {
    pub fn active(&self) -> bool {
        self.key_held || self.mouse_held || self.script_held || self.midi_held || self.latched
    }

    /// Get (or create) this pad's `slot`-th effect for a layer, and include it this tick.
    fn layer_fx(&mut self, layer: u64, slot: usize, kind: EffectKind) -> &mut Effect {
        let list = self.layer_fx.entry(layer).or_default();
        get_slot(list, slot, kind)
    }

    fn master_fx(&mut self, slot: usize, kind: EffectKind) -> &mut Effect {
        get_slot(&mut self.master_fx, slot, kind)
    }
}

fn get_slot(list: &mut Vec<Effect>, slot: usize, kind: EffectKind) -> &mut Effect {
    while list.len() <= slot {
        let mut e = Effect::new(kind);
        e.enabled = false;
        list.push(e);
    }
    if list[slot].kind != kind {
        list[slot] = Effect::new(kind);
    }
    list[slot].enabled = true;
    &mut list[slot]
}

/// What a punch routine can see and change this tick.
struct Ctx<'a> {
    comp: &'a mut Composition,
    /// Indices of layers that are drawing something, bottom first.
    active: Vec<usize>,
    beat: f64,
    bpm: f32,
    /// Envelope 0..1.
    a: f32,
    /// Beats since the pad was pressed.
    held: f64,
}

fn hash(a: i64, b: i64) -> f32 {
    let mut z = (a as u64).wrapping_mul(0x9e3779b97f4a7c15) ^ (b as u64).wrapping_mul(0xbf58476d1ce4e5b9);
    z ^= z >> 31;
    z = z.wrapping_mul(0x94d049bb133111eb);
    z ^= z >> 29;
    (z >> 40) as f32 / (1u64 << 24) as f32
}

#[derive(Default)]
pub struct Punch {
    pub pads: Vec<Pad>,
}

impl Punch {
    pub fn new() -> Self {
        Self {
            pads: (0..PUNCHES.len()).map(|_| Pad::default()).collect(),
        }
    }

    /// Advance envelopes and apply every active pad. Call once per tick, before the
    /// composition ticks and renders.
    pub fn update(&mut self, comp: &mut Composition, clock: Clock, dt: f64) {
        // Clear last tick's overrides.
        comp.master_perf = 1.0;
        for l in &mut comp.layers {
            l.perf = Default::default();
            for c in l.clips.iter_mut().flatten() {
                c.perf = Default::default();
            }
        }
        let active: Vec<usize> = (0..comp.layers.len())
            .filter(|&i| comp.layer_audible(i) && !comp.layers[i].draw_list().is_empty())
            .collect();

        for (i, pad) in self.pads.iter_mut().enumerate() {
            let on = pad.active();
            if on && !pad.was_active {
                pad.start_beat = clock.beat;
                pad.rolls.clear();
            }
            pad.was_active = on;
            let rate = if on { dt as f32 / ATTACK_SECS } else { dt as f32 / RELEASE_SECS };
            pad.amount = if on { (pad.amount + rate).min(1.0) } else { (pad.amount - rate).max(0.0) };

            for list in pad.layer_fx.values_mut() {
                for e in list {
                    e.enabled = false;
                }
            }
            for e in &mut pad.master_fx {
                e.enabled = false;
            }
            if pad.amount <= 0.0 {
                continue;
            }
            let mut ctx = Ctx {
                comp: &mut *comp,
                active: active.clone(),
                beat: clock.beat,
                bpm: clock.bpm,
                a: pad.amount,
                held: clock.beat - pad.start_beat,
            };
            apply(i, pad, &mut ctx);
        }
    }

    /// Temporary effects to run after a layer's own chain this tick.
    pub fn layer_effects(&mut self, layer_id: u64) -> Vec<&mut Effect> {
        let mut out = Vec::new();
        for pad in &mut self.pads {
            if pad.amount > 0.0
                && let Some(list) = pad.layer_fx.get_mut(&layer_id)
            {
                out.extend(list.iter_mut().filter(|e| e.enabled));
            }
        }
        out
    }

    /// Temporary effects to run after the master chain this tick.
    pub fn master_effects(&mut self) -> Vec<&mut Effect> {
        let mut out = Vec::new();
        for pad in &mut self.pads {
            if pad.amount > 0.0 {
                out.extend(pad.master_fx.iter_mut().filter(|e| e.enabled));
            }
        }
        out
    }

    /// Drop effect instances for layers that no longer exist.
    pub fn retain_layers(&mut self, comp: &Composition) {
        let ids: Vec<u64> = comp.layers.iter().map(|l| l.id).collect();
        for pad in &mut self.pads {
            pad.layer_fx.retain(|id, _| ids.contains(id));
        }
    }
}

fn apply(index: usize, pad: &mut Pad, c: &mut Ctx) {
    let a = c.a;
    let n = c.active.len();
    let step16 = (c.beat * 4.0).floor() as i64;
    let step8 = (c.beat * 2.0).floor() as i64;
    let beat_n = c.beat.floor() as i64;
    match index {
        // Stutter: beat repeat, getting tighter the longer it's held.
        0 => {
            if a < 0.5 {
                return;
            }
            let beats = if c.held < 2.0 { 0.5 } else if c.held < 4.0 { 0.25 } else { 0.125 };
            for &li in &c.active {
                if let Some(clip) = c.comp.layers[li].active_clip_mut().filter(|c| c.is_timeline()) {
                    let anchor = *pad.rolls.entry(clip.id).or_insert(clip.position);
                    clip.perf.roll = Some(Roll { anchor, start_beat: pad.start_beat, beats });
                }
            }
        }
        // Chase: one layer at a time on 1/16 notes.
        1 => {
            if n > 1 {
                let k = step16.rem_euclid(n as i64) as usize;
                for (j, &li) in c.active.iter().enumerate() {
                    if j != k {
                        c.comp.layers[li].perf.opacity *= 1.0 - a;
                    }
                }
            }
        }
        // Mirror split: alternate mirror axes per layer, kaleidoscope on top.
        2 => {
            for (j, &li) in c.active.iter().enumerate() {
                let id = c.comp.layers[li].id;
                pad.layer_fx(id, 0, EffectKind::Mirror).set("mode", if j % 2 == 0 { 0.0 } else { 2.0 });
                if j + 1 == n {
                    let k = pad.layer_fx(id, 1, EffectKind::Kaleidoscope);
                    k.set("segments", 6.0);
                    k.set("rotation °", ((c.beat * 45.0) % 360.0 - 180.0) as f32);
                }
            }
        }
        // Pump: per-layer zoom kicks, phase-shifted.
        3 => {
            for (j, &li) in c.active.iter().enumerate() {
                let phase = (c.beat + j as f64 / n.max(1) as f64).fract() as f32;
                let kick = (-6.0 * phase).exp();
                let p = &mut c.comp.layers[li].perf;
                p.scale *= 1.0 + 0.25 * a * kick;
                p.rotate += a * 4.0 * kick * if j % 2 == 0 { 1.0 } else { -1.0 };
            }
        }
        // Tape stop: clips grind to a halt over one beat; picture sags.
        4 => {
            let t = (c.held as f32).min(1.0);
            let speed = 1.0 - t;
            for &li in &c.active {
                if let Some(clip) = c.comp.layers[li].active_clip_mut() {
                    clip.perf.speed *= (1.0 + (speed - 1.0) * a) as f64;
                }
            }
            let col = pad.master_fx(0, EffectKind::Color);
            col.set("saturation", 1.0 - 0.7 * t * a);
            col.set("brightness", 1.0 - 0.5 * t * a);
            let w = pad.master_fx(1, EffectKind::Wave);
            w.set("amplitude", 0.03 * t * a);
            w.set("frequency", 3.0);
            w.set("speed", 2.0);
            w.set("direction", 1.0);
        }
        // Reverse: clips backwards, time-split colour, hue flip.
        5 => {
            for &li in &c.active {
                if let Some(clip) = c.comp.layers[li].active_clip_mut() {
                    clip.perf.reverse = a > 0.5;
                }
            }
            let s = pad.master_fx(0, EffectKind::RgbSplit);
            s.set("delay (frames)", 3.0);
            s.set("amount", a);
            pad.master_fx(1, EffectKind::Color).set("hue", 0.5 * a);
        }
        // Echo build: 1/16-note trails that grow while held.
        6 => {
            let sixteenth_frames = (60.0 * 60.0 / c.bpm.max(1.0) / 4.0).round().clamp(1.0, 10.0);
            let e = pad.master_fx(0, EffectKind::Echo);
            e.set("spacing (frames)", sixteenth_frames);
            e.set("amount", a * (0.4 + c.held as f32 / 4.0).min(1.0));
            e.set("decay", (0.5 + c.held as f32 / 8.0).min(0.92));
            e.set("mode", 0.0);
        }
        // Riser: a two-bar build with an accelerating strobe.
        7 => {
            let b = (c.held as f32 / 8.0).min(1.0) * a;
            pad.master_fx(0, EffectKind::Transform).set("zoom", 1.0 + 0.6 * b);
            pad.master_fx(1, EffectKind::Blur).set("radius (px)", 12.0 * b);
            let col = pad.master_fx(2, EffectKind::Color);
            col.set("brightness", 1.0 + 0.6 * b);
            col.set("saturation", 1.0 - 0.5 * b);
            if c.held >= 2.0 {
                let st = pad.master_fx(3, EffectKind::Strobe);
                st.set("rate (beats)", if c.held < 4.0 { 2.0 } else if c.held < 6.0 { 1.0 } else { 0.0 });
                st.set("on time", 0.5);
                st.set("mode", 1.0);
            }
        }
        // Strobe split: even layers on the beat, odd ones on the off-beat.
        8 => {
            for (j, &li) in c.active.iter().enumerate() {
                if (step8 + j as i64) % 2 != 0 {
                    c.comp.layers[li].perf.opacity *= 1.0 - a;
                }
            }
        }
        // Kaleido spin.
        9 => {
            let k = pad.master_fx(0, EffectKind::Kaleidoscope);
            k.set("segments", [6.0, 8.0, 12.0, 4.0][beat_n.rem_euclid(4) as usize]);
            k.set("rotation °", ((c.beat * 90.0) % 360.0 - 180.0) as f32);
        }
        // Tunnel: master feedback, flipping direction each bar.
        10 => {
            let dir = if (beat_n / 4) % 2 == 0 { 1.0 } else { -1.0 };
            let f = pad.master_fx(0, EffectKind::Feedback);
            f.set("feedback", 0.95 * a);
            f.set("copy scale", 0.92);
            f.set("rotate °", 4.0 * dir);
            f.set("hue / pass", 0.015);
            f.set("input mode", 2.0);
        }
        // Fractal bloom on the top layer.
        11 => {
            if let Some(&top) = c.active.last() {
                let id = c.comp.layers[top].id;
                let f = pad.layer_fx(id, 0, EffectKind::Feedback);
                f.set("feedback", a);
                f.set("copies (monitors)", 3.0);
                f.set("copy scale", 0.5);
                f.set("spread", 0.5);
                f.set("rotate °", 0.0);
                f.set("twist ° / copy", (20.0 * (c.beat * std::f64::consts::FRAC_PI_4).sin()) as f32);
                f.set("hue / pass", 0.02);
                f.set("input mode", 2.0);
                for &li in &c.active[..n - 1] {
                    c.comp.layers[li].perf.opacity *= 1.0 - 0.5 * a;
                }
            }
        }
        // Glitch: random per-layer jumps, a random layer pixelates, CRT master.
        12 => {
            for (j, &li) in c.active.iter().enumerate() {
                let r = hash(step16, j as i64);
                if r < 0.5 {
                    let p = &mut c.comp.layers[li].perf;
                    p.x += (hash(step16, 100 + j as i64) - 0.5) * 0.12 * a;
                    p.y += (hash(step16, 200 + j as i64) - 0.5) * 0.12 * a;
                }
            }
            if n > 0 {
                let k = (hash(step8, 7) * n as f32) as usize % n;
                let id = c.comp.layers[c.active[k]].id;
                pad.layer_fx(id, 0, EffectKind::Pixelate).set("pixel size", 8.0 + 24.0 * hash(step8, 9));
            }
            let crt = pad.master_fx(0, EffectKind::Crt);
            crt.set("RGB shift (px)", 6.0 * a);
            crt.set("scanlines", 0.3 * a);
            crt.set("noise", 0.25 * a);
            crt.set("curvature", 0.0);
            crt.set("vignette", 0.2 * a);
        }
        // Invert flip: alternating layers invert, swapping every beat.
        13 => {
            if a < 0.5 {
                return;
            }
            for (j, &li) in c.active.iter().enumerate() {
                if (beat_n + j as i64) % 2 == 0 {
                    let id = c.comp.layers[li].id;
                    pad.layer_fx(id, 0, EffectKind::Color).set("invert", 1.0);
                }
            }
        }
        // Wash out: whiteout over a bar.
        14 => {
            let b = (c.held as f32 / 4.0).min(1.0) * a;
            pad.master_fx(0, EffectKind::Blur).set("radius (px)", 20.0 * b);
            let col = pad.master_fx(1, EffectKind::Color);
            col.set("brightness", 1.0 + 1.5 * b);
            col.set("saturation", 1.0 - b);
            col.set("contrast", 1.0 - 0.5 * b);
        }
        // Trance gate: 16-step chop of the master.
        15 => {
            const PATTERN: [bool; 16] = [
                true, false, true, false, true, true, false, true, true, false, true, false, true, true, true, false,
            ];
            if !PATTERN[step16.rem_euclid(16) as usize] {
                c.comp.master_perf *= 1.0 - a;
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::Clip;

    fn comp() -> Composition {
        let mut c = Composition::new(3, 2);
        for l in 0..3 {
            c.set_clip(l, 0, Clip::generator(l));
            c.layers[l].launch(0);
        }
        c
    }

    fn clock(beat: f64) -> Clock {
        Clock::new(beat, beat / 2.0, 120.0)
    }

    /// Every pad can run for a few bars without panicking (e.g. on a wrong parameter label)
    /// and fully releases afterwards.
    #[test]
    fn every_pad_runs_and_releases() {
        for (i, def) in PUNCHES.iter().enumerate() {
            let mut c = comp();
            let mut p = Punch::new();
            p.pads[i].key_held = true;
            for t in 0..600 {
                p.update(&mut c, clock(t as f64 / 30.0), 1.0 / 60.0);
                let _ = p.master_effects();
                for l in &c.layers {
                    let _ = p.layer_effects(l.id);
                }
            }
            p.pads[i].key_held = false;
            for t in 600..700 {
                p.update(&mut c, clock(t as f64 / 30.0), 1.0 / 60.0);
            }
            assert_eq!(p.pads[i].amount, 0.0, "{} didn't release", def.name);
            assert!(p.master_effects().is_empty());
            assert_eq!(c.master_perf, 1.0);
            for l in &c.layers {
                assert_eq!(l.perf.opacity, 1.0);
                assert_eq!(l.perf.scale, 1.0);
            }
        }
    }

    #[test]
    fn chase_shows_one_layer_per_sixteenth() {
        let mut c = comp();
        let mut p = Punch::new();
        p.pads[1].key_held = true;
        for t in 0..10 {
            p.update(&mut c, clock(t as f64 / 120.0), 1.0 / 60.0);
        }
        let visible = c.layers.iter().filter(|l| l.perf.opacity > 0.5).count();
        assert_eq!(visible, 1);
    }

    #[test]
    fn mirror_split_targets_layers_separately() {
        let mut c = comp();
        let mut p = Punch::new();
        p.pads[2].key_held = true;
        p.update(&mut c, clock(0.0), 1.0 / 60.0);
        let modes: Vec<f32> = c.layers.iter().map(|l| p.layer_effects(l.id)[0].params[0].value).collect();
        assert_eq!(modes, [0.0, 2.0, 0.0], "alternating mirror axes");
        assert_eq!(p.layer_effects(c.layers[2].id).len(), 2, "top layer also gets the kaleidoscope");
    }
}
