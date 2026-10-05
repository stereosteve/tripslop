//! Sound reactivity: an audio input (a device, or a WAV file for scripts) analysed every tick
//! into a few smoothed 0..1 values, plus a spectrum and a waveform for shaders.
//!
//! * `level`: overall loudness
//! * `bass`, `mid`, `high`: band energies (20–150 Hz, 150–2000 Hz, 2–12 kHz)
//! * `kick`: jumps to 1 on a bass onset and decays over ~150 ms
//! * `centroid`: spectral brightness (0 = dark, 1 = bright)
//!
//! Each value has auto-gain (it's divided by its own slowly decaying peak) and attack/release
//! smoothing, so it sits in 0..1 without constant tweaking, whatever the input level.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use realfft::RealFftPlanner;
use realfft::num_complex::Complex;

/// Samples analysed per tick (~43 ms at 48 kHz).
const WINDOW: usize = 2048;
/// Bands in the spectrum texture, and samples in the waveform one.
pub const TEX_LEN: usize = 512;
/// Below this RMS the input counts as silence (auto-gain doesn't amplify noise).
const FLOOR: f32 = 1e-3;

/// The analysed values for one tick. `Default` is silence.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Levels {
    pub level: f32,
    pub bass: f32,
    pub mid: f32,
    pub high: f32,
    pub kick: f32,
    pub centroid: f32,
    /// An input is running (shaders fall back to the tempo clock otherwise).
    pub active: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Band {
    Level,
    Bass,
    Mid,
    High,
    Kick,
}

impl Band {
    pub const ALL: [Band; 5] = [Band::Level, Band::Bass, Band::Mid, Band::High, Band::Kick];
    pub fn name(self) -> &'static str {
        match self {
            Band::Level => "Level",
            Band::Bass => "Bass",
            Band::Mid => "Mid",
            Band::High => "High",
            Band::Kick => "Kick",
        }
    }
}

impl Levels {
    pub fn get(&self, band: Band) -> f32 {
        match band {
            Band::Level => self.level,
            Band::Bass => self.bass,
            Band::Mid => self.mid,
            Band::High => self.high,
            Band::Kick => self.kick,
        }
    }
}

/// Where samples come from.
enum Source {
    Off,
    /// A live input; the callback appends mono samples to the shared buffer.
    Device { name: String, _stream: cpal::Stream, buf: Arc<Mutex<VecDeque<f32>>> },
    /// A WAV file, read at the simulation clock (deterministic, for scripts).
    File { name: String, samples: Vec<f32>, start: f64 },
}

pub struct Audio {
    source: Source,
    sample_rate: f32,
    analyzer: Analyzer,
    /// The latest analysis.
    pub levels: Levels,
    /// Log-spaced spectrum, 0..1 per band (30 Hz → 16 kHz).
    pub spectrum: Vec<f32>,
    /// The latest samples, -1..1.
    pub waveform: Vec<f32>,
    window: Vec<f32>,
}

impl Default for Audio {
    fn default() -> Self {
        Self {
            source: Source::Off,
            sample_rate: 48000.0,
            analyzer: Analyzer::new(48000.0),
            levels: Levels::default(),
            spectrum: vec![0.0; TEX_LEN],
            waveform: vec![0.0; TEX_LEN],
            window: vec![0.0; WINDOW],
        }
    }
}

/// Names of the input devices.
pub fn input_devices() -> Vec<String> {
    let host = cpal::default_host();
    let Ok(devices) = host.input_devices() else { return Vec::new() };
    devices.filter_map(|d| d.description().ok().map(|desc| desc.name().to_string())).collect()
}

impl Audio {
    /// What's being listened to, for the UI.
    pub fn source_name(&self) -> Option<&str> {
        match &self.source {
            Source::Off => None,
            Source::Device { name, .. } | Source::File { name, .. } => Some(name),
        }
    }

    pub fn is_file(&self) -> bool {
        matches!(self.source, Source::File { .. })
    }

    pub fn stop(&mut self) {
        self.source = Source::Off;
        self.levels = Levels::default();
        self.spectrum.fill(0.0);
        self.waveform.fill(0.0);
    }

    /// Listen to an input device (`None`: the default one).
    pub fn open_device(&mut self, name: Option<&str>) -> Result<(), String> {
        self.stop();
        let host = cpal::default_host();
        let device = match name {
            None => host.default_input_device().ok_or("no audio input device")?,
            Some(n) => host
                .input_devices()
                .map_err(|e| e.to_string())?
                .find(|d| d.description().is_ok_and(|desc| desc.name() == n))
                .ok_or_else(|| format!("no audio input {n:?}"))?,
        };
        let label = device.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "input".into());
        let supported = device.default_input_config().map_err(|e| e.to_string())?;
        let config = supported.config();
        let channels = config.channels.max(1) as usize;
        let buf = Arc::new(Mutex::new(VecDeque::with_capacity(WINDOW * 4)));
        let err = |e: cpal::Error| eprintln!("audio input: {e}");
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream::<f32, _, _>(config.clone(), feeder(buf.clone(), channels), err, None),
            cpal::SampleFormat::I16 => device.build_input_stream::<i16, _, _>(config.clone(), feeder(buf.clone(), channels), err, None),
            cpal::SampleFormat::I32 => device.build_input_stream::<i32, _, _>(config.clone(), feeder(buf.clone(), channels), err, None),
            cpal::SampleFormat::U16 => device.build_input_stream::<u16, _, _>(config.clone(), feeder(buf.clone(), channels), err, None),
            other => return Err(format!("unsupported sample format {other:?}")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        self.set_rate(config.sample_rate as f32);
        self.source = Source::Device { name: label, _stream: stream, buf };
        Ok(())
    }

    /// Analyse a WAV file in step with the clock, starting at simulation time `now`.
    pub fn open_file(&mut self, path: &Path, now: f64) -> Result<(), String> {
        self.stop();
        let mut reader = hound::WavReader::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let spec = reader.spec();
        let channels = spec.channels.max(1) as usize;
        let raw: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader.samples::<f32>().filter_map(Result::ok).collect(),
            hound::SampleFormat::Int => {
                let scale = 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32;
                reader.samples::<i32>().filter_map(Result::ok).map(|s| s as f32 * scale).collect()
            }
        };
        let samples = raw.chunks(channels).map(|c| c.iter().sum::<f32>() / channels as f32).collect();
        self.set_rate(spec.sample_rate as f32);
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.source = Source::File { name, samples, start: now };
        Ok(())
    }

    fn set_rate(&mut self, rate: f32) {
        self.sample_rate = rate;
        self.analyzer = Analyzer::new(rate);
    }

    /// Analyse the latest window; call once per tick. `now` is the simulation time.
    pub fn tick(&mut self, now: f64, dt: f32) {
        match &self.source {
            Source::Off => return,
            Source::Device { buf, .. } => {
                let mut b = buf.lock().unwrap();
                // Keep only what the next window needs.
                let excess = b.len().saturating_sub(WINDOW);
                b.drain(..excess);
                let n = b.len();
                self.window.fill(0.0);
                for (i, s) in b.iter().enumerate() {
                    self.window[WINDOW - n + i] = *s;
                }
            }
            Source::File { samples, start, .. } => {
                let end = ((now - start).max(0.0) * self.sample_rate as f64) as usize;
                self.window.fill(0.0);
                for i in 0..WINDOW {
                    if let Some(s) = (end + i).checked_sub(WINDOW).and_then(|j| samples.get(j)) {
                        self.window[i] = *s;
                    }
                }
            }
        }
        self.levels = self.analyzer.process(&self.window, dt, &mut self.spectrum);
        // Scaled by the loudness peak, so the waveform fills the range at any input level.
        let gain = 0.5 / self.analyzer.peak[0];
        for (i, w) in self.waveform.iter_mut().enumerate() {
            *w = (self.window[WINDOW - TEX_LEN + i] * gain).clamp(-1.0, 1.0);
        }
    }
}

/// Input callback: mix to mono and append to the shared buffer.
fn feeder<T: cpal::SizedSample>(buf: Arc<Mutex<VecDeque<f32>>>, channels: usize) -> impl FnMut(&[T], &cpal::InputCallbackInfo) + Send + 'static
where
    f32: cpal::FromSample<T>,
{
    move |data: &[T], _| {
        let mut b = buf.lock().unwrap();
        for frame in data.chunks(channels) {
            let s: f32 = frame.iter().map(|x| <f32 as cpal::FromSample<T>>::from_sample_(*x)).sum::<f32>() / channels as f32;
            b.push_back(s);
        }
        // Never grow without bound if ticks stop (e.g. a hidden window).
        let excess = b.len().saturating_sub(WINDOW * 8);
        b.drain(..excess);
    }
}

/// Turns windows of samples into `Levels`. Separate from `Audio` so it can be tested on
/// generated signals.
pub struct Analyzer {
    rate: f32,
    fft: Arc<dyn realfft::RealToComplex<f32>>,
    hann: Vec<f32>,
    scratch: Vec<f32>,
    bins: Vec<Complex<f32>>,
    /// Auto-gain peaks: level, bass, mid, high, spectrum.
    peak: [f32; 5],
    smooth: Levels,
    /// Slow average of the bass energy, for onsets.
    bass_avg: f32,
    prev_bass: f32,
    since_kick: f32,
}

impl Analyzer {
    pub fn new(rate: f32) -> Self {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(WINDOW);
        let hann = (0..WINDOW).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / WINDOW as f32).cos()).collect();
        let bins = fft.make_output_vec();
        Self {
            rate,
            fft,
            hann,
            scratch: vec![0.0; WINDOW],
            bins,
            peak: [FLOOR; 5],
            smooth: Levels::default(),
            bass_avg: 0.0,
            prev_bass: 0.0,
            since_kick: 1.0,
        }
    }

    /// Analyse one window (`WINDOW` samples, newest last), `dt` seconds after the last one.
    /// Fills `spectrum` with `TEX_LEN` log-spaced bands.
    pub fn process(&mut self, window: &[f32], dt: f32, spectrum: &mut [f32]) -> Levels {
        let rms = (window[WINDOW / 2..].iter().map(|s| s * s).sum::<f32>() / (WINDOW / 2) as f32).sqrt();
        for (o, (s, w)) in self.scratch.iter_mut().zip(window.iter().zip(&self.hann)) {
            *o = s * w;
        }
        let _ = self.fft.process(&mut self.scratch, &mut self.bins);
        let hz_per_bin = self.rate / WINDOW as f32;
        let mag = |b: &Complex<f32>| b.norm() * 4.0 / WINDOW as f32;
        let band = |lo: f32, hi: f32| {
            let (a, b) = ((lo / hz_per_bin) as usize, ((hi / hz_per_bin) as usize).min(self.bins.len() - 1));
            let e: f32 = self.bins[a.max(1)..=b.max(a.max(1))].iter().map(|c| mag(c).powi(2)).sum();
            e.sqrt()
        };
        let raw = [rms, band(20.0, 150.0), band(150.0, 2000.0), band(2000.0, 12000.0)];

        // Spectral centroid, on a log-frequency scale.
        let (mut wsum, mut fsum) = (0.0, 0.0);
        for (i, c) in self.bins.iter().enumerate().skip(1) {
            let m = mag(c);
            wsum += m;
            fsum += m * ((i as f32 * hz_per_bin).max(30.0) / 30.0).ln();
        }
        let centroid = if wsum > 0.0 { (fsum / wsum / (16000.0f32 / 30.0).ln()).clamp(0.0, 1.0) } else { 0.0 };

        // Auto-gain: each value over its own peak, which halves in ~10 s of quiet.
        let decay = 0.5f32.powf(dt / 10.0);
        let silent = rms < FLOOR;
        let mut norm = [0.0f32; 4];
        for i in 0..4 {
            self.peak[i] = (self.peak[i] * decay).max(raw[i]).max(FLOOR);
            norm[i] = if silent { 0.0 } else { (raw[i] / self.peak[i]).clamp(0.0, 1.0) };
        }

        // Kick: the bass jumping well above its recent average.
        self.since_kick += dt;
        self.bass_avg += (raw[1] - self.bass_avg) * (1.0 - (-dt / 0.4).exp());
        let onset = !silent && raw[1] > self.bass_avg * 1.4 && raw[1] > self.prev_bass && raw[1] > self.peak[1] * 0.3 && self.since_kick > 0.12;
        self.prev_bass = raw[1];
        let mut kick = self.smooth.kick * (-dt / 0.15).exp();
        if onset {
            kick = 1.0;
            self.since_kick = 0.0;
        }

        // Fast attack, slower release.
        let follow = |cur: f32, target: f32| {
            let tau = if target > cur { 0.015 } else { 0.15 };
            cur + (target - cur) * (1.0 - (-dt / tau).exp())
        };
        let s = &mut self.smooth;
        s.level = follow(s.level, norm[0]);
        s.bass = follow(s.bass, norm[1]);
        s.mid = follow(s.mid, norm[2]);
        s.high = follow(s.high, norm[3]);
        s.kick = kick;
        s.centroid = follow(s.centroid, if silent { 0.0 } else { centroid });
        s.active = true;

        // Log-spaced spectrum for shaders, over its own peak.
        let (lo, hi) = (30.0f32.ln(), 16000.0f32.ln());
        let n = spectrum.len();
        let mut top = 0.0f32;
        for (k, out) in spectrum.iter_mut().enumerate() {
            let f0 = (lo + (hi - lo) * k as f32 / n as f32).exp();
            let f1 = (lo + (hi - lo) * (k + 1) as f32 / n as f32).exp();
            let (a, b) = ((f0 / hz_per_bin) as usize, ((f1 / hz_per_bin) as usize).min(self.bins.len() - 1));
            let m = self.bins[a.max(1)..=b.max(a.max(1))].iter().map(mag).fold(0.0, f32::max);
            top = top.max(m);
            *out = m;
        }
        self.peak[4] = (self.peak[4] * decay).max(top).max(FLOOR);
        for v in spectrum.iter_mut() {
            *v = if silent { 0.0 } else { (*v / self.peak[4]).sqrt().clamp(0.0, 1.0) };
        }
        *s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48000.0;
    const DT: f32 = 1.0 / 60.0;

    /// Run `seconds` of a generated signal through the analyzer a tick at a time; returns the
    /// levels after each tick.
    fn run(seconds: f32, signal: impl Fn(f32) -> f32) -> Vec<Levels> {
        let mut a = Analyzer::new(RATE);
        let mut spectrum = vec![0.0; TEX_LEN];
        let ticks = (seconds / DT) as usize;
        (1..=ticks)
            .map(|t| {
                let end = (t as f32 * DT * RATE) as usize;
                let window: Vec<f32> = (0..WINDOW).map(|i| (end + i).checked_sub(WINDOW).map_or(0.0, |j| signal(j as f32 / RATE))).collect();
                a.process(&window, DT, &mut spectrum)
            })
            .collect()
    }

    fn sine(hz: f32, amp: f32) -> impl Fn(f32) -> f32 {
        move |t| amp * (std::f32::consts::TAU * hz * t).sin()
    }

    #[test]
    fn bands_follow_frequency() {
        let low = *run(1.0, sine(60.0, 0.5)).last().unwrap();
        assert!(low.bass > 0.8 && low.high < 0.2, "{low:?}");
        let high = *run(1.0, sine(5000.0, 0.5)).last().unwrap();
        assert!(high.high > 0.8 && high.bass < 0.2, "{high:?}");
        assert!(high.centroid > low.centroid + 0.3, "brighter sound, higher centroid: {low:?} {high:?}");
    }

    #[test]
    fn auto_gain_ignores_input_level() {
        let loud = *run(1.0, sine(60.0, 0.8)).last().unwrap();
        let quiet = *run(1.0, sine(60.0, 0.02)).last().unwrap();
        assert!((loud.bass - quiet.bass).abs() < 0.1, "{loud:?} vs {quiet:?}");
    }

    #[test]
    fn silence_is_zero() {
        let l = *run(0.5, |_| 0.0).last().unwrap();
        assert_eq!((l.level, l.bass, l.mid, l.high, l.kick), (0.0, 0.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn kicks_fire_on_bass_hits() {
        // A 60 Hz thump every half second (120 BPM), 80 ms long.
        let kick = |t: f32| {
            let p = t % 0.5;
            if p < 0.08 { (std::f32::consts::TAU * 60.0 * t).sin() * (1.0 - p / 0.08) } else { 0.0 }
        };
        let levels = run(4.0, kick);
        let hits = levels.windows(2).filter(|w| w[1].kick == 1.0 && w[0].kick < 1.0).count();
        assert!((7..=9).contains(&hits), "{hits} kicks in 4 s at 120 BPM");
        // Between hits the envelope falls away.
        assert!(levels.iter().any(|l| l.kick < 0.2));
    }
}
