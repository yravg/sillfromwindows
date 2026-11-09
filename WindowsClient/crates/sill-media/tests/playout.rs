//! Ports of Tests/checks/audio-playout/main.swift (cited per test as "Swift: <section>") on the
//! timing rules, plus the engine (kept packets, timeline ring, render) and the public `Playout`.

use sill_media::playout::engine::{Engine, Ring};
use sill_media::playout::timing::{self, Part, Reset, Timing, MARGIN};
use sill_media::playout::{resample, Playout};

fn near(a: f64, b: f64, eps: f64) -> bool {
    (a - b).abs() < eps
}

fn fresh(epoch: i64, fpp: usize, priming: usize) -> Timing {
    let mut m = Timing::new();
    m.set_away(false);
    assert!(m.format(epoch, fpp, priming, &[]));
    m
}

fn pk(m: &mut Timing, seq: u32, stamp: f64, seg: bool, now: f64) -> timing::Plan {
    m.packet(m.epoch().unwrap(), seq, stamp, seg, None, now)
}

// Swift: "The need, the floor, the cover, the picture's lag: by hand" (format lines)
#[test]
fn format_new_epoch_same_epoch_ignored() {
    let mut m = Timing::new();
    assert!(m.format(3, 480, 240, &[9]), "a format: a new epoch");
    assert!(
        !m.format(3, 480, 240, &[9]),
        "same epoch and cookie: ignored"
    );
    assert!(m.format(3, 480, 240, &[8]), "another cookie: a new decoder");
    assert!(m.format(4, 512, 256, &[8]), "another epoch");
}

// Swift: by hand: cover's start, the first packet's floor, the need
#[test]
fn cover_start_floor_and_need() {
    let mut m = Timing::new();
    m.format(4, 512, 256, &[8]);
    m.latency(0.011, 0.005, 1.0 / 60.0, 60.0);
    assert!(
        m.floor().is_none() && near(m.cover(0.0), 0.040, 1e-12),
        "no packet: no floor, 40 ms"
    );
    m.set_away(true);
    assert!(near(m.cover(0.0), 0.150, 1e-12), "away: 150 ms");
    m.set_away(false);
    assert!(near(m.cover(0.0), 0.040, 1e-12), "home: 40 ms");
    pk(&mut m, 0, 99.995, true, 100.0);
    assert!(
        near(m.floor().unwrap(), 0.005, 1e-9),
        "the first packet sets the floor"
    );
    let need = m.need(100.0);
    assert!(
        near(
            need,
            0.040 + 512.0 / 48_000.0 + 0.005 + 0.011 + 0.002,
            1e-12
        ),
        "the need: {need}"
    );
    pk(&mut m, 1, 100.5 - 0.0053, false, 100.6);
    assert!(
        near(m.cover(100.6), 0.040, 1e-12),
        "a packet 100 ms over in the first 2 s leaves the cover"
    );
    pk(&mut m, 2, 102.3 - 0.035, false, 102.3);
    assert!(
        near(m.cover(102.3), 0.030, 1e-9),
        "past 2 s a packet 30 ms over makes the cover 30 ms"
    );
    pk(&mut m, 3, 102.4 - 0.0051, false, 102.4);
    assert!(
        near(m.cover(102.4), 0.030, 1e-9),
        "the largest of the window stays"
    );
}

// Swift: by hand: the cover's bounds, home and away
#[test]
fn cover_bounds() {
    let mut lone = fresh(1, 480, 240);
    pk(&mut lone, 0, 9.990, true, 10.0);
    pk(&mut lone, 1, 12.5 - 0.011, false, 12.5);
    assert!(
        near(lone.cover(12.5), 0.020, 1e-9),
        "1 ms: home's lower bound 20 ms"
    );
    pk(&mut lone, 2, 12.6 - 0.310, false, 12.6);
    assert!(
        near(lone.cover(12.6), 0.150, 1e-9),
        "300 ms home: held at 150"
    );
    lone.set_away(true);
    assert!(near(lone.cover(12.6), 0.300, 1e-9), "the same away: 300 ms");
    pk(&mut lone, 3, 12.7 - 0.910, false, 12.7);
    assert!(
        near(lone.cover(12.7), 0.600, 1e-9),
        "900 ms away: held at 600"
    );
}

// Swift: by hand: rule 6, a packet that waited behind the engine starting gives when it came
#[test]
fn waited_packet_takes_its_arrival() {
    let mut w = fresh(1, 480, 240);
    pk(&mut w, 0, 9.990, true, 10.0);
    pk(&mut w, 1, 12.5 - 0.011, false, 12.5);
    w.packet(1, 2, 12.6 - 0.011, false, Some(12.6), 12.68);
    assert!(near(w.cover(12.68), 0.020, 1e-9) && near(w.floor().unwrap(), 0.010, 1e-9));
}

// Swift: by hand: the floor window, 10 s of 1 s buckets, kept when nothing comes
#[test]
fn floor_window() {
    let mut fl = fresh(1, 480, 240);
    pk(&mut fl, 0, 0.990, true, 1.0);
    for k in 1..=9u32 {
        pk(&mut fl, k, k as f64 + 1.5 - 0.015, false, k as f64 + 1.5);
    }
    assert!(
        near(fl.floor().unwrap(), 0.010, 1e-9),
        "the smallest of the last 10 s"
    );
    pk(&mut fl, 10, 11.5 - 0.015, false, 11.5);
    assert!(
        near(fl.floor().unwrap(), 0.015, 1e-9),
        "10 s on, the smallest has left"
    );
}

// Swift: by hand: the picture's lag (median of 2 s of frames), the guard, a still window, none
#[test]
fn picture_lag_and_guard() {
    let mut pic = fresh(1, 480, 240);
    pic.latency(0.010, 0.005, 1.0 / 60.0, 60.0);
    pk(&mut pic, 0, 9.980, true, 10.0);
    for (k, raw) in [0.030, 0.010, 0.050, 0.012, 0.040].iter().enumerate() {
        let a = 10.0 + k as f64 * 0.1;
        pic.frame(a - raw, a);
    }
    let lag = pic.picture_lag(10.5).unwrap();
    assert!(near(lag, 0.030 - 0.020 + 1.5 / 60.0 + 0.004, 1e-9), "{lag}");
    assert!(
        near(pic.delay(10.5, Some(lag)), pic.need(10.5), 1e-12),
        "the need decides while larger"
    );
    for k in 0..5 {
        let a = 12.1 + k as f64 * 0.1;
        pic.frame(a - 0.300, a);
    }
    let late = pic.picture_lag(12.5).unwrap();
    assert!(near(late, 0.300 - 0.020 + 1.5 / 60.0 + 0.004, 1e-9));
    assert!(
        near(pic.delay(12.5, Some(late)), late + 1.0 / 60.0, 1e-12),
        "the guard decides"
    );
    assert!(
        near(pic.picture_lag(20.0).unwrap(), late, 1e-9),
        "a still window keeps its last lag"
    );
    assert!(
        Timing::new().picture_lag(0.0).is_none(),
        "no frame, no floor: no lag"
    );
}

// Not in the Swift (this API's): the lag given directly replaces the frames' median
#[test]
fn lag_override() {
    let mut m = fresh(1, 480, 0);
    pk(&mut m, 0, 9.98, true, 10.0);
    m.set_lag_override(Some(0.300));
    assert!(near(m.picture_lag(10.0).unwrap(), 0.300, 1e-12));
    assert!(near(
        m.delay(10.0, m.picture_lag(10.0)),
        0.300 + 1.0 / 60.0,
        1e-12
    ));
    m.set_lag_override(None);
    assert!(m.picture_lag(10.0).is_none());
}

fn steady(m: &mut Timing, from: u32, to: u32, t0: f64, transit: f64) -> (f64, Vec<timing::Output>) {
    let mut t = t0;
    let mut outs = vec![];
    for k in from..to {
        // The mapping's reading every packet: sample n goes out at t0-aligned time.
        let o = m.played(((t - 100.0) * 48_000.0).round(), t, t);
        outs.push(o);
        let plan = m.packet(1, k, t - transit, k == 0, None, t);
        outs.push(plan.output);
        t += 0.010;
    }
    (t, outs)
}

// Swift: "Rule 10: a packet past the hand's deadline, before its timer ran"
#[test]
fn rule10_packet_past_deadline() {
    let mut m = fresh(1, 480, 0);
    m.latency(0.010, 0.005, 1.0 / 60.0, 60.0);
    m.played(0.0, 100.0, 100.0);
    let mut t = 100.0;
    for k in 0..50u32 {
        pk(&mut m, k, t - 0.023, k == 0, t);
        t += 0.010;
    }
    let d = m.deadline().unwrap();
    let now = d + 0.004;
    let out = pk(&mut m, 50, t - 0.023, false, now).output;
    let tail = out
        .schedules
        .iter()
        .find(|s| s.part == Part::Tail && s.seq == 49);
    assert!(
        tail.map(|s| s.fade_out).unwrap_or(false),
        "the tail in hand goes out faded"
    );
    let next: Vec<_> = out.schedules.iter().filter(|s| s.seq == 50).collect();
    assert!(
        !out.discards.is_empty()
            || (!next.is_empty() && next.iter().all(|s| s.at.is_some() && s.fade_in)),
        "placed by time, faded in (or late)"
    );
    assert!(
        next.iter()
            .all(|s| s.goes_out - now >= 0.005 + MARGIN - 1e-9),
        "never less than IO + 2 ms ahead"
    );
}

// Swift: "Fades and a frame gained or lost"
#[test]
fn fades_and_frame_change() {
    let mut x = vec![1.0f32; 480];
    timing::fade(&mut x, true, timing::fade_frames());
    assert!(
        x[0] < 0.001 && x[239] > 0.99 && x[240] == 1.0 && x[..240].windows(2).all(|w| w[0] <= w[1]),
        "fade in 5 ms"
    );
    let mut y = vec![1.0f32; 480];
    timing::fade(&mut y, false, timing::fade_frames());
    assert!(y[479] < 0.001 && y[240] > 0.99 && y[239] == 1.0, "fade out");
    let mut short = vec![1.0f32; 100];
    timing::fade(&mut short, true, timing::fade_frames());
    assert!(
        short[0] < 0.01 && short[99] > 0.99,
        "a short packet fades over all of it"
    );

    let sine: Vec<f32> = (0..480)
        .map(|i| (2.0 * std::f64::consts::PI * 440.0 * i as f64 / 48_000.0).sin() as f32)
        .collect();
    let (mut l, mut r) = (sine.clone(), sine.clone());
    timing::change(&mut l, &mut r, 1);
    assert!(l.len() == 481 && r.len() == 481 && l == r, "a frame gained");
    let jump = l
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    let natural = sine
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    assert!(jump <= natural * 1.01, "no larger step than the sine's own");
    let (mut l2, mut r2) = (sine.clone(), sine.clone());
    timing::change(&mut l2, &mut r2, -1);
    assert!(l2.len() == 479 && r2.len() == 479);
    let jump2 = l2
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    assert!(jump2 <= natural * 1.6);
    let first_diff = (0..sine.len().min(l.len()))
        .find(|&i| sine[i] != l[i])
        .unwrap();
    assert!(
        (120..=360).contains(&first_diff),
        "in the middle half (at {first_diff})"
    );
    let q = (120..360).min_by(|&a, &b| {
        (sine[a].abs() + sine[a + 1].abs())
            .partial_cmp(&(sine[b].abs() + sine[b + 1].abs()))
            .unwrap()
    });
    assert_eq!(first_diff, q.unwrap() + 1, "at its quietest point");
    let mut n0 = sine.clone();
    let mut n1 = sine.clone();
    timing::change(&mut n0, &mut n1, 0);
    assert_eq!(n0, sine);
}

// Swift: Scenarios, "a steady link at home: nothing late after the first 2 s"
#[test]
fn steady_link_nothing_late_error_settles() {
    for (ol, io) in [
        (0.010, 0.005),
        (0.180, 0.005),
        (2.0, 0.005),
        (0.010, 0.023),
        (0.180, 0.023),
    ] {
        let mut m = fresh(1, 480, 240);
        m.latency(ol, io, 1.0 / 60.0, 60.0);
        let mut late_after_2s = 0;
        let mut t = 100.0f64;
        let mut placed = 0;
        let mut worst = 0.0f64;
        for k in 0..3000u32 {
            m.played(((t - 100.0) * 48_000.0).round(), t, t);
            let before = m.take_second(t).late;
            let plan = m.packet(
                1,
                k,
                t - (0.003 + 0.0005 * (t * 7.3).sin()),
                k == 0,
                None,
                t,
            );
            let _ = before;
            placed += plan.output.schedules.len();
            if t > 102.0 {
                let s = m.take_second(t);
                late_after_2s += s.late;
                if t > 120.0 {
                    if let Some(e) = m.last_error() {
                        worst = worst.max(e.abs());
                    }
                }
            }
            t += 0.010;
        }
        assert_eq!(late_after_2s, 0, "output {ol}, io {io}");
        assert!(placed > 2600);
        assert!(worst < 0.001, "its error under 1 ms once settled: {worst}");
    }
}

// Swift: rule 7, duplicates are dropped before they count
#[test]
fn duplicates_dropped_before_counting() {
    let mut m = fresh(1, 480, 0);
    m.played(0.0, 100.0, 100.0);
    let a = pk(&mut m, 0, 99.99, true, 100.0);
    assert!(a.decode);
    let floor = m.floor();
    let b = pk(&mut m, 0, 50.0, true, 100.001);
    assert!(!b.decode && b.id.is_none(), "a duplicate is not decoded");
    assert_eq!(m.floor(), floor, "and never counts toward the floor");
    assert_eq!(m.take_second(101.0).duplicates, 1);
}

// Swift: rule 8, a skipped seq: the packet after the gap is decoded and dropped whole, the next fades in
#[test]
fn gap_drops_then_fades_in() {
    let mut m = fresh(1, 480, 0);
    m.played(0.0, 100.0, 100.0);
    let (t, _) = steady(&mut m, 0, 5, 100.0, 0.02);
    let p = m.packet(1, 7, t - 0.02, false, None, t); // 5 and 6 skipped
    assert!(p.decode && p.reset_first && p.drop_front == 480 && p.id.is_none());
    let q = m.packet(1, 8, t + 0.01 - 0.02, false, None, t + 0.01);
    let first = q.output.schedules.iter().find(|s| s.seq == 8);
    assert!(
        first
            .map(|s| s.fade_in && s.at.is_some())
            .unwrap_or(q.output.discards.len() == 1),
        "placed by time, faded in"
    );
}

// Swift: rule 9, a segment start drops the priming frames
#[test]
fn segment_start_drops_priming() {
    let mut m = fresh(1, 480, 240);
    m.played(0.0, 100.0, 100.0);
    let p = pk(&mut m, 0, 99.99, true, 100.0);
    assert!(p.reset_first && p.drop_front == 240 && p.id.is_some());
}

// Swift: rule 13, a clock step (a second of packets all above the floor) restarts the floor
#[test]
fn clock_step_restarts_floor() {
    let mut m = fresh(1, 480, 0);
    m.latency(0.010, 0.005, 1.0 / 60.0, 60.0);
    let mut t = 100.0;
    let mut k = 0u32;
    while t < 103.0 {
        m.played((t - 100.0) * 48_000.0, t, t);
        pk(&mut m, k, t - 0.003, k == 0, t);
        k += 1;
        t += 0.010;
    }
    assert!(near(m.floor().unwrap(), 0.003, 1e-6));
    // The Mac's clock steps back 300 ms: every packet now reads 300 ms more than the floor.
    let jumps_before = m.take_second(t).jumps;
    assert_eq!(jumps_before, 0);
    while t < 105.0 {
        m.played((t - 100.0) * 48_000.0, t, t);
        pk(&mut m, k, t - 0.303, false, t);
        k += 1;
        t += 0.010;
    }
    assert!(
        m.floor().unwrap() > 0.2,
        "the floor started again from the new packets"
    );
    let s = m.take_second(t);
    assert!(s.jumps >= 1, "closed by a jump");
}

// Swift: rule 13 below, a packet 20 ms under the floor
#[test]
fn packet_under_floor_steps() {
    let mut m = fresh(1, 480, 0);
    m.played(0.0, 100.0, 100.0);
    let mut t = 100.0;
    for k in 0..20u32 {
        pk(&mut m, k, t - 0.050, k == 0, t);
        t += 0.010;
    }
    pk(&mut m, 20, t - 0.010, false, t); // 40 ms under
    assert!(near(m.floor().unwrap(), 0.010, 1e-6));
}

// Swift: reset(.playback) keeps the floor and the dedupe, throws away placement; .session everything
#[test]
fn resets() {
    let mut m = fresh(1, 480, 0);
    m.played(0.0, 100.0, 100.0);
    let mut t = 100.0;
    for k in 0..10u32 {
        pk(&mut m, k, t - 0.02, k == 0, t);
        t += 0.01;
    }
    assert!(m.holding());
    let out = m.reset(Reset::Playback);
    assert!(!out.discards.is_empty() && !m.holding());
    assert!(m.floor().is_some(), "the floor stays");
    assert!(
        pk(&mut m, 9, t - 0.02, false, t).id.is_none(),
        "dedupe stays"
    );
    m.reset(Reset::Session);
    assert!(m.floor().is_none() && m.epoch().is_none());
}

// Swift: mute: each packet still counts for the floor, none is decoded
#[test]
fn muted_packets_count_but_do_not_decode() {
    let mut m = fresh(1, 480, 0);
    m.set_muted(true);
    let p = pk(&mut m, 0, 9.99, true, 10.0);
    assert!(!p.decode && p.id.is_none());
    assert!(near(m.floor().unwrap(), 0.01, 1e-9));
}

// ---- engine and the public type (not in the Swift)

#[test]
fn ring_writes_gaps_and_past() {
    let mut r = Ring::new();
    r.write_at(2, &[1.0, 2.0], &[1.0, 2.0]);
    assert_eq!(r.end(), 4);
    assert_eq!(r.len(), 4, "a gap of silence before");
    r.write_at(-1, &[9.0, 9.0, 3.0], &[9.0, 9.0, 3.0]); // before base 0: first frames skipped
    assert_eq!(r.len(), 4);
}

fn sine(frames: usize, phase: usize) -> Vec<f32> {
    let mut v = Vec::new();
    for i in 0..frames {
        let s =
            (2.0 * std::f64::consts::PI * 440.0 * (i + phase) as f64 / 48_000.0).sin() as f32 * 0.5;
        v.push(s);
        v.push(s);
    }
    v
}

#[test]
fn engine_plays_a_steady_stream_after_its_delay_then_goes_silent() {
    let mut e = Engine::new();
    e.timing.set_away(false);
    e.latency(0.010, 0.005);
    let wall = 1.79e9;
    let mut now = 0.0;
    let mut heard = Vec::<f32>::new();
    let mut buf = vec![0.0f32; 480 * 2];
    for k in 0..200usize {
        let samples = sine(480, k * 480);
        e.push(&samples, 2, wall + k as f64 * 0.01, k == 0, now);
        e.render(&mut buf, 2, 48_000, now, 0.010);
        heard.extend_from_slice(&buf);
        now += 0.010;
    }
    let loud = heard.iter().filter(|x| x.abs() > 0.01).count();
    assert!(loud > 100 * 480, "most of the stream is audible: {loud}");
    // Nothing is audible before the delay: the first 40 ms are silence.
    assert!(
        heard[..2 * 1920].iter().all(|x| x.abs() < 1e-6),
        "silent until due"
    );
    // Underrun: no more packets, the ring drains to silence.
    for _ in 0..30 {
        e.render(&mut buf, 2, 48_000, now, 0.010);
        now += 0.010;
    }
    assert!(buf.iter().all(|x| *x == 0.0), "an underrun is silence");
}

#[test]
fn engine_resamples_to_a_44100_device() {
    let mut e = Engine::new();
    e.timing.set_away(false);
    let wall = 1.79e9;
    let mut now = 0.0;
    let mut buf = vec![0.0f32; 441 * 2];
    let mut total_loud = 0;
    for k in 0..100usize {
        e.push(&sine(480, k * 480), 2, wall + k as f64 * 0.01, k == 0, now);
        e.render(&mut buf, 2, 44_100, now, 0.010);
        total_loud += buf.iter().filter(|x| x.abs() > 0.01).count();
        now += 0.010;
    }
    assert!(
        total_loud > 30 * 441,
        "audible through a 44.1 kHz device: {total_loud}"
    );
}

#[test]
fn engine_mono_input_and_mute() {
    let mut e = Engine::new();
    e.timing.set_away(false);
    let mono: Vec<f32> = sine(480, 0).iter().step_by(2).copied().collect();
    let wall = 1.79e9;
    let mut now = 0.0;
    let mut buf = vec![0.0f32; 480 * 2];
    let mut loud_before = 0;
    for k in 0..50usize {
        e.push(&mono, 1, wall + k as f64 * 0.01, k == 0, now);
        e.render(&mut buf, 2, 48_000, now, 0.010);
        loud_before += buf.iter().filter(|x| x.abs() > 0.01).count();
        now += 0.010;
    }
    assert!(loud_before > 0, "mono is duplicated to both channels");
    e.set_muted(true);
    for k in 50..60usize {
        e.push(&mono, 1, wall + k as f64 * 0.01, false, now);
        e.render(&mut buf, 2, 48_000, now, 0.010);
        assert!(buf.iter().all(|x| *x == 0.0), "muted: silence");
        now += 0.010;
    }
    assert!(e.timing.floor().is_some(), "muted packets still count");
}

#[test]
fn resample_lengths() {
    let x = vec![0.5f32; 441 * 2];
    let y = resample(&x, 2, 44_100, 48_000);
    assert_eq!(y.len() / 2, 480);
    assert!(y.iter().all(|v| (*v - 0.5).abs() < 1e-6));
}

#[test]
fn playout_public_api_silent_sink() {
    let mut p = Playout::new(48_000, 2);
    p.set_picture_lag_ms(80.0);
    p.set_muted(false);
    p.set_away(true);
    for k in 0..20usize {
        p.push(&sine(480, k * 480), 1.79e9 + k as f64 * 0.01, k == 0);
    }
    p.set_muted(true);
    p.push(&sine(480, 0), 1.79e9 + 0.3, false);
    p.stop();
    p.push(&sine(480, 0), 1.79e9 + 0.4, true);
}
