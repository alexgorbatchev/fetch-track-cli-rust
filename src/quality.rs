use crate::{Error, Result};
use rustfft::{FftPlanner, num_complex::Complex};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Quality {
    pub bandwidth_hz: u32,
    pub bandwidth_rating: String,
    pub peak_dbfs: f64,
    pub rms_dbfs: f64,
    pub clipping_warning: bool,
    pub low_bandwidth_warning: bool,
    pub sub_bass_dbfs: f64,
    pub kick_bass_dbfs: f64,
    pub suggested_gain_db: f64,
}
fn db(value: f64) -> f64 {
    20.0 * value.max(1e-6).log10()
}
pub fn analyze(samples: &[f32], rate: usize) -> Result<Quality> {
    if samples.len() < 4 || rate < 2 || samples.iter().any(|v| !v.is_finite()) {
        return Err(Error::Invalid("invalid or empty PCM samples".into()));
    }
    let peak = samples
        .iter()
        .map(|v| f64::from(v.abs()))
        .fold(0.0, f64::max);
    let rms =
        (samples.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / samples.len() as f64).sqrt();
    let size = 8192.min(samples.len().next_power_of_two());
    let mut power = vec![0.0; size / 2 + 1];
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(size);
    let window: Vec<f32> = (0..size)
        .map(|n| 0.5 - 0.5 * (std::f32::consts::TAU * n as f32 / (size - 1) as f32).cos())
        .collect();
    let scale = 2.0 / window.iter().map(|v| f64::from(*v)).sum::<f64>();
    let mut frames = 0;
    for start in (0..samples.len()).step_by(size / 2) {
        if start + size > samples.len() && frames > 0 {
            break;
        }
        let mut buffer: Vec<Complex<f32>> = window
            .iter()
            .enumerate()
            .map(|(i, w)| Complex::new(samples.get(start + i).copied().unwrap_or(0.0) * w, 0.0))
            .collect();
        fft.process(&mut buffer);
        for (p, v) in power.iter_mut().zip(buffer) {
            *p += f64::from(v.norm_sqr()) * scale * scale;
        }
        frames += 1;
    }
    for p in &mut power {
        *p /= f64::from(frames);
    }
    let max = power.iter().copied().fold(0.0, f64::max);
    let highest = power
        .iter()
        .rposition(|p| *p > max * 0.001 && *p > 1e-12)
        .unwrap_or(0);
    let bandwidth_hz = (highest * rate / size) as u32;
    let insufficient = rms < 1e-6;
    let rating = if insufficient {
        "Insufficient signal"
    } else if bandwidth_hz < 16000 {
        "Limited bandwidth"
    } else if bandwidth_hz < 19000 {
        "Standard"
    } else {
        "Wide bandwidth"
    };
    let tone = |freq: usize| {
        let index = freq * size / rate;
        let lo = index.saturating_sub(1);
        let hi = (index + 2).min(power.len());
        db(power[lo.min(hi)..hi]
            .iter()
            .copied()
            .fold(0.0, f64::max)
            .sqrt())
    };
    Ok(Quality {
        bandwidth_hz: if insufficient { 0 } else { bandwidth_hz },
        bandwidth_rating: rating.into(),
        peak_dbfs: db(peak),
        rms_dbfs: db(rms),
        clipping_warning: peak >= 0.999,
        low_bandwidth_warning: !insufficient && bandwidth_hz < 16000,
        sub_bass_dbfs: tone(45),
        kick_bass_dbfs: tone(90),
        suggested_gain_db: if insufficient {
            0.0
        } else {
            (-12.0 - db(rms)).min(-db(peak))
        },
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Structure {
    pub radio_edit: bool,
    pub extended: bool,
    pub description: String,
    pub keywords: Vec<String>,
    pub intro_beats: bool,
    pub outro_beats: bool,
}
pub fn structure(title: &str, duration: f64) -> Structure {
    let title = title.to_lowercase();
    let radio: Vec<_> = [
        "radio edit",
        "short edit",
        "radio version",
        "single edit",
        "short version",
        "tiktok edit",
    ]
    .into_iter()
    .filter(|s| title.contains(s))
    .collect();
    let extended: Vec<_> = [
        "original mix",
        "extended mix",
        "club mix",
        "dub mix",
        "extended version",
        "12 inch",
        "vip mix",
    ]
    .into_iter()
    .filter(|s| title.contains(s))
    .collect();
    let description = if !radio.is_empty() {
        "Radio Edit (Short)"
    } else if !extended.is_empty() {
        "Original / Extended DJ Mix"
    } else if duration <= 240.0 {
        "Radio Edit / Short Track (<= 4.0 mins)"
    } else if duration >= 300.0 {
        "Full Length Track (> 5.0 mins)"
    } else {
        "Standard Track (4.0 - 5.0 mins)"
    };
    Structure {
        radio_edit: !radio.is_empty() || (duration <= 240.0 && extended.is_empty()),
        extended: !extended.is_empty() || (duration >= 300.0 && radio.is_empty()),
        description: description.into(),
        keywords: radio
            .into_iter()
            .chain(extended)
            .map(str::to_owned)
            .collect(),
        intro_beats: duration > 210.0,
        outro_beats: duration > 210.0,
    }
}
