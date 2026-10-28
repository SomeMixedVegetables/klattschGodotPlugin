// Low-level DSP primitives used by the formant synth core.

use std::f64::consts::PI;

use crate::js;

// Constant-skirt-gain bandpass biquad (RBJ Audio EQ Cookbook)
// Coefficients are recomputed only when frequency or bandwidth changes
#[derive(Clone, Debug)]
pub struct BandpassBiquad {
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
    last_f: f64,
    last_bw: f64,
}

impl Default for BandpassBiquad {
    fn default() -> Self {
        Self::new()
    }
}

impl BandpassBiquad {
    pub fn new() -> Self {
        Self {
            x1: 0.0, x2: 0.0,
            y1: 0.0, y2: 0.0,
            b0: 0.0, b1: 0.0, b2: 0.0,
            a1: 0.0, a2: 0.0,
            last_f: -1.0, last_bw: -1.0,
        }
    }

    pub fn set_freq(&mut self, f: f64, bw: f64, sr: f64) {
        if f == self.last_f && bw == self.last_bw {
            return;
        }
        self.last_f = f;
        self.last_bw = bw;
        let f = js::max(40.0, js::min(sr * 0.45, f));
        let bw = js::max(20.0, bw);
        let w0 = 2.0 * PI * f / sr;
        let cosw0 = js::cos(w0);
        let sinw0 = js::sin(w0);
        let q = f / bw;
        let alpha = sinw0 / (2.0 * q);
        let a0 = 1.0 + alpha;
        self.b0 = alpha / a0;
        self.b1 = 0.0;
        self.b2 = -alpha / a0;
        self.a1 = -2.0 * cosw0 / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    pub fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }
}

pub const DEFAULT_EFFORT: f64 = 0.5;

// Derivative of the Rosenberg glottal pulse. Phase normalized to [0, 1).
// Peak |value| is pi / (2·Tn) ~= 9.82, so we divide by 10 to keep amplitude
// near unity.
//
// `effort` (0..1) controls the pulse shape: 0 is lax/breathy (longer Tp,
// gentler closure), 1 is tense (shorter Tp, sharper closure)
pub fn glottal_pulse(phase: f64, effort: f64) -> f64 {
    let e = if effort < 0.0 { 0.0 } else if effort > 1.0 { 1.0 } else { effort };
    let tp = 0.5 - e * 0.2; // 0.5 (lax) -> 0.3 (tense)
    let tn = 0.25 - e * 0.17; // 0.25 (lax) -> 0.08 (tense)
    const NORM: f64 = 0.1;
    if phase < tp {
        return NORM * 0.5 * (PI / tp) * js::sin(PI * phase / tp);
    }
    if phase < tp + tn {
        return -NORM * (PI / (2.0 * tn)) * js::sin(PI * (phase - tp) / (2.0 * tn));
    }
    0.0
}

// 32-bit xorshift LFSR
pub fn xorshift(state: i32) -> i32 {
    let mut x = state as u32;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    x as i32
}

// Soft-clip with a linear region up to ±0.85 and a smooth knee
pub fn soft_clip(x: f64) -> f64 {
    const T: f64 = 0.85;
    let a = if x < 0.0 { -x } else { x };
    if a <= T {
        return x;
    }
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let excess = a - T;
    sign * (T + (1.0 - T) * excess / (excess + 1.0))
}
