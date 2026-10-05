//! Test / automation scripts: `tripslop --script file.tripslop`.
//!
//! A script is a list of timed commands run against the live app, e.g.
//!
//! ```text
//! # comments start with #
//! demo                          # time 0: load the demo set
//! at 2b   launch-scene 2        # at beat 2
//! at +4b  hold Q 4b             # 4 beats later, hold the Stutter pad for 4 beats
//! at 600  snapshot target/script-out/end.png
//! assert 2/feedback/copies == 3
//! quit
//! ```
//!
//! Times: a number of frames (60 per second), `Ns` seconds, `Nb` beats, or `+N<unit>`
//! relative to the previous line's time (same unit). Lines without `at` run at the same time
//! as the line before. Layers and columns are 1-based, like the UI. Failed asserts make the
//! process exit with status 1.

use std::path::PathBuf;

use crate::composition::{Crossfade, FadeCurve, Side};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum When {
    Frame(u64),
    Seconds(f64),
    Beat(f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    /// Equal within 1e-3.
    Approx,
}

impl Op {
    pub fn check(self, a: f64, b: f64) -> bool {
        match self {
            Op::Eq => a == b,
            Op::Ne => a != b,
            Op::Lt => a < b,
            Op::Le => a <= b,
            Op::Gt => a > b,
            Op::Ge => a >= b,
            Op::Approx => (a - b).abs() <= 1e-3,
        }
    }
    fn parse(s: &str) -> Option<Op> {
        Some(match s {
            "==" => Op::Eq,
            "!=" => Op::Ne,
            "<" => Op::Lt,
            "<=" => Op::Le,
            ">" => Op::Gt,
            ">=" => Op::Ge,
            "~=" => Op::Approx,
            _ => return None,
        })
    }
}

/// Something a script can read: a parameter path or a piece of runtime state.
#[derive(Clone, Debug, PartialEq)]
pub enum Query {
    /// `layer/param`, `layer/effect/param`, `layer/clip/param`, `master/...` (see `App::with_param`).
    Param(String),
    /// Playhead (frames) of the layer's active clip.
    Playhead(usize),
    /// 1-based column playing on a layer, 0 if none.
    Active(usize),
    /// Envelope (0..1) of a punch pad.
    Pad(usize),
    /// Number of compile errors of the shader clip in a cell.
    Errors(usize, usize),
    Layers,
    Bpm,
    Beat,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Demo,
    LaunchScene(usize),
    Launch(usize, usize),
    Stop(usize),
    Load(usize, usize, PathBuf),
    Generator(usize, usize, String),
    Shader(usize, usize, String),
    Camera(usize, usize, u32),
    /// `None` = master.
    AddEffect(Option<usize>, String),
    Set(String, f32),
    Bpm(f32),
    Quantize(String),
    Crossfade(Crossfade, Option<FadeCurve>),
    Side(usize, Side),
    Play(bool),
    PadDown(usize),
    PadUp(usize),
    PadLatch(usize, bool),
    Select(usize, usize),
    OpenEditor(usize, usize),
    Tab(String),
    Snapshot(PathBuf),
    Screenshot(PathBuf),
    Record(bool),
    Print(Query),
    Assert(Query, Op, f64),
    Quit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub at: When,
    pub cmd: Cmd,
    /// Source line, for messages.
    pub line: usize,
}

/// Split a line into words, honouring "double quotes".
fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any {
        out.push(cur);
    }
    out
}

fn parse_when(s: &str, prev: When) -> Result<When, String> {
    let (rel, s) = match s.strip_prefix('+') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let (num, unit) = match s.char_indices().find(|(_, c)| c.is_ascii_alphabetic()) {
        Some((i, _)) => (&s[..i], &s[i..]),
        None => (s, "f"),
    };
    let v: f64 = num.parse().map_err(|_| format!("bad time {s:?}"))?;
    let when = match unit {
        "f" => When::Frame(v as u64),
        "s" => When::Seconds(v),
        "b" => When::Beat(v),
        _ => return Err(format!("bad time unit {unit:?} (use f, s or b)")),
    };
    if !rel {
        return Ok(when);
    }
    match (prev, when) {
        (When::Frame(a), When::Frame(b)) => Ok(When::Frame(a + b)),
        (When::Seconds(a), When::Seconds(b)) => Ok(When::Seconds(a + b)),
        (When::Beat(a), When::Beat(b)) => Ok(When::Beat(a + b)),
        _ => Err("a relative time must use the same unit as the line before".into()),
    }
}

/// `Q`..`K` (the pad's key) or a 1-based pad number.
pub fn parse_pad(s: &str) -> Result<usize, String> {
    if let Ok(n) = s.parse::<usize>() {
        if (1..=crate::punch::PUNCHES.len()).contains(&n) {
            return Ok(n - 1);
        }
        return Err(format!("no pad {n}"));
    }
    crate::punch::PUNCHES
        .iter()
        .position(|p| format!("{:?}", p.key).eq_ignore_ascii_case(s) || p.name.eq_ignore_ascii_case(s))
        .ok_or_else(|| format!("no pad {s:?} (use its key, name or number)"))
}

fn num<T: std::str::FromStr>(s: Option<&String>, what: &str) -> Result<T, String> {
    let s = s.ok_or_else(|| format!("missing {what}"))?;
    s.parse().map_err(|_| format!("bad {what} {s:?}"))
}

/// 1-based index -> 0-based.
fn idx(s: Option<&String>, what: &str) -> Result<usize, String> {
    let n: usize = num(s, what)?;
    n.checked_sub(1).ok_or_else(|| format!("{what} is 1-based"))
}

/// Everything in `w` is the query (parameter paths may contain spaces, quoted or not).
fn parse_query(w: &[String]) -> Result<Query, String> {
    let first = w.first().ok_or("missing what to read")?;
    Ok(match first.as_str() {
        "playhead" => Query::Playhead(idx(w.get(1), "layer")?),
        "active" => Query::Active(idx(w.get(1), "layer")?),
        "pad" => Query::Pad(parse_pad(&w[1..].join(" "))?),
        "errors" => Query::Errors(idx(w.get(1), "layer")?, idx(w.get(2), "column")?),
        "layers" => Query::Layers,
        "bpm" => Query::Bpm,
        "beat" => Query::Beat,
        _ => Query::Param(w.join(" ")),
    })
}

fn parse_cmd(w: &[String]) -> Result<Vec<Cmd>, String> {
    let rest = || w[1..].join(" ");
    let one = |c: Cmd| Ok(vec![c]);
    match w[0].as_str() {
        "demo" => one(Cmd::Demo),
        "launch-scene" => one(Cmd::LaunchScene(idx(w.get(1), "scene")?)),
        "launch" => one(Cmd::Launch(idx(w.get(1), "layer")?, idx(w.get(2), "column")?)),
        "stop" => one(Cmd::Stop(idx(w.get(1), "layer")?)),
        "load" => one(Cmd::Load(idx(w.get(1), "layer")?, idx(w.get(2), "column")?, PathBuf::from(w.get(3).ok_or("missing path")?))),
        "generator" => one(Cmd::Generator(idx(w.get(1), "layer")?, idx(w.get(2), "column")?, w.get(3).ok_or("missing generator name")?.clone())),
        "shader" => one(Cmd::Shader(idx(w.get(1), "layer")?, idx(w.get(2), "column")?, w.get(3).ok_or("missing template name")?.clone())),
        "camera" => one(Cmd::Camera(idx(w.get(1), "layer")?, idx(w.get(2), "column")?, num(w.get(3), "device")?)),
        "add-effect" => {
            let target = w.get(1).ok_or("missing layer (or master)")?;
            let layer = if target == "master" { None } else { Some(idx(w.get(1), "layer")?) };
            one(Cmd::AddEffect(layer, w.get(2).ok_or("missing effect name")?.clone()))
        }
        // The value is the last word; the path is everything before it (spaces allowed).
        "set" => {
            if w.len() < 3 {
                return Err("set needs a parameter path and a value".into());
            }
            one(Cmd::Set(w[1..w.len() - 1].join(" "), num(w.last(), "value")?))
        }
        "bpm" => one(Cmd::Bpm(num(w.get(1), "bpm")?)),
        "quantize" => one(Cmd::Quantize(w.get(1).ok_or("missing off/beat/bar")?.clone())),
        "crossfade" => {
            let mode = match w.get(1).map(String::as_str) {
                Some("bank") => Crossfade::Bank,
                Some("layer") => Crossfade::Layer,
                _ => return Err("crossfade needs bank / layer".into()),
            };
            let curve = match w.get(2).map(String::as_str) {
                None => None,
                Some("linear") => Some(FadeCurve::Linear),
                Some("smooth") => Some(FadeCurve::Smooth),
                Some("cut") => Some(FadeCurve::Cut),
                Some(c) => return Err(format!("crossfade curve {c:?}: use linear / smooth / cut")),
            };
            one(Cmd::Crossfade(mode, curve))
        }
        "side" => {
            let side = match w.get(2).map(String::as_str) {
                Some("a" | "A") => Side::A,
                Some("b" | "B") => Side::B,
                Some("off" | "both") => Side::Both,
                _ => return Err("side needs a layer and a / b / off".into()),
            };
            one(Cmd::Side(idx(w.get(1), "layer")?, side))
        }
        "play" => one(Cmd::Play(true)),
        "pause" => one(Cmd::Play(false)),
        "pad" => {
            let p = parse_pad(w.get(1).ok_or("missing pad")?)?;
            match w.get(2).map(String::as_str) {
                Some("down") => one(Cmd::PadDown(p)),
                Some("up") => one(Cmd::PadUp(p)),
                Some("latch") => one(Cmd::PadLatch(p, true)),
                Some("unlatch") => one(Cmd::PadLatch(p, false)),
                _ => Err("pad needs down / up / latch / unlatch".into()),
            }
        }
        "select" => one(Cmd::Select(idx(w.get(1), "layer")?, idx(w.get(2), "column")?)),
        "open-editor" => one(Cmd::OpenEditor(idx(w.get(1), "layer")?, idx(w.get(2), "column")?)),
        "tab" => one(Cmd::Tab(w.get(1).ok_or("missing layer/composition")?.clone())),
        "snapshot" => one(Cmd::Snapshot(PathBuf::from(w.get(1).ok_or("missing path")?))),
        "screenshot" => one(Cmd::Screenshot(PathBuf::from(w.get(1).ok_or("missing path")?))),
        "record" => match w.get(1).map(String::as_str) {
            Some("start") => one(Cmd::Record(true)),
            Some("stop") => one(Cmd::Record(false)),
            _ => Err("record needs start / stop".into()),
        },
        "print" => one(Cmd::Print(parse_query(&w[1..])?)),
        // assert WHAT OP VALUE, where WHAT may contain spaces.
        "assert" => {
            if w.len() < 4 {
                return Err("assert needs: what, operator, value".into());
            }
            let op = Op::parse(&w[w.len() - 2]).ok_or("assert needs an operator: == != < <= > >= ~=")?;
            let v: f64 = num(w.last(), "expected value")?;
            one(Cmd::Assert(parse_query(&w[1..w.len() - 2])?, op, v))
        }
        "quit" => one(Cmd::Quit),
        other => Err(format!("unknown command {other:?} in {:?}", rest())),
    }
}

/// Parse a whole script. Errors are `line: message`.
pub fn parse(src: &str) -> Result<Vec<Event>, String> {
    let mut events = Vec::new();
    let mut now = When::Frame(0);
    for (n, raw) in src.lines().enumerate() {
        let line = n + 1;
        let text = raw.split('#').next().unwrap_or("");
        let mut w = words(text);
        if w.is_empty() {
            continue;
        }
        let err = |m: String| format!("line {line}: {m}");
        if w[0] == "at" {
            let t = w.get(1).ok_or_else(|| err("`at` needs a time".into()))?;
            now = parse_when(t, now).map_err(err)?;
            w.drain(..2);
            if w.is_empty() {
                continue;
            }
        }
        // `hold PAD DURATION` = pad down now, pad up DURATION later.
        if w[0] == "hold" {
            let pad = parse_pad(w.get(1).ok_or_else(|| err("missing pad".into()))?).map_err(err)?;
            let dur = w.get(2).ok_or_else(|| err("missing duration, e.g. 4b".into()))?;
            let end = parse_when(&format!("+{}", dur.trim_start_matches('+')), now).map_err(err)?;
            events.push(Event { at: now, cmd: Cmd::PadDown(pad), line });
            events.push(Event { at: end, cmd: Cmd::PadUp(pad), line });
            continue;
        }
        for cmd in parse_cmd(&w).map_err(err)? {
            events.push(Event { at: now, cmd, line });
        }
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_times_commands_and_holds() {
        let ev = parse(
            "# comment\n\
             demo\n\
             at 2b launch-scene 2   # trailing comment\n\
             at +4b hold Q 4b\n\
             at 600 snapshot \"out dir/end.png\"\n\
             assert 2/feedback/copies ~= 3\n\
             assert playhead 1 < 10.5\n\
             at 10s quit\n",
        )
        .unwrap();
        let cmds: Vec<(When, Cmd)> = ev.into_iter().map(|e| (e.at, e.cmd)).collect();
        assert_eq!(
            cmds,
            vec![
                (When::Frame(0), Cmd::Demo),
                (When::Beat(2.0), Cmd::LaunchScene(1)),
                (When::Beat(6.0), Cmd::PadDown(0)),
                (When::Beat(10.0), Cmd::PadUp(0)),
                (When::Frame(600), Cmd::Snapshot("out dir/end.png".into())),
                (When::Frame(600), Cmd::Assert(Query::Param("2/feedback/copies".into()), Op::Approx, 3.0)),
                (When::Frame(600), Cmd::Assert(Query::Playhead(0), Op::Lt, 10.5)),
                (When::Seconds(10.0), Cmd::Quit),
            ]
        );
    }

    #[test]
    fn paths_with_spaces_need_no_quotes() {
        let ev = parse("set 1/shape/spin y 0.5\nassert 1/clip/pattern speed ~= 0\nprint Fractal/feedback/rotate °\n").unwrap();
        assert_eq!(ev[0].cmd, Cmd::Set("1/shape/spin y".into(), 0.5));
        assert_eq!(ev[1].cmd, Cmd::Assert(Query::Param("1/clip/pattern speed".into()), Op::Approx, 0.0));
        assert_eq!(ev[2].cmd, Cmd::Print(Query::Param("Fractal/feedback/rotate °".into())));
    }

    #[test]
    fn errors_name_the_line() {
        assert_eq!(parse("demo\nlaunch 0 1\n").unwrap_err(), "line 2: layer is 1-based");
        assert!(parse("at 2b x\n").unwrap_err().starts_with("line 1: unknown command"));
        assert!(parse("at 2b demo\nat +3 demo\n").unwrap_err().contains("same unit"));
        assert!(parse("pad Z down\n").unwrap_err().contains("no pad"));
    }

    #[test]
    fn crossfade_and_sides() {
        let ev = parse("crossfade bank smooth\ncrossfade layer\nside 2 b\nside 1 off\n").unwrap();
        assert_eq!(ev[0].cmd, Cmd::Crossfade(Crossfade::Bank, Some(FadeCurve::Smooth)));
        assert_eq!(ev[1].cmd, Cmd::Crossfade(Crossfade::Layer, None));
        assert_eq!(ev[2].cmd, Cmd::Side(1, Side::B));
        assert_eq!(ev[3].cmd, Cmd::Side(0, Side::Both));
        assert!(parse("crossfade bank wobbly\n").unwrap_err().contains("curve"));
        assert!(parse("side 1 c\n").is_err());
    }

    #[test]
    fn pads_by_key_name_or_number() {
        assert_eq!(parse_pad("q"), Ok(0));
        assert_eq!(parse_pad("Tape stop"), Ok(4));
        assert_eq!(parse_pad("16"), Ok(15));
    }
}
