use crate::candidate::Candidate;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub struct HumanProgress(Option<indicatif::ProgressBar>);
impl HumanProgress {
    pub fn new(agent: bool, verbose: bool) -> Self {
        let bar = (!agent && !verbose).then(|| {
            let bar = indicatif::ProgressBar::new_spinner();
            bar.set_style(
                indicatif::ProgressStyle::with_template("{spinner} {msg}")
                    .expect("valid progress template")
                    .tick_strings(&["-", "\\", "|", "/"]),
            );
            bar.enable_steady_tick(std::time::Duration::from_millis(100));
            bar
        });
        Self(bar)
    }
    pub fn phase(&self, message: &str) {
        if let Some(bar) = &self.0 {
            bar.set_message(message.to_owned());
        }
    }
}
impl Drop for HumanProgress {
    fn drop(&mut self) {
        if let Some(bar) = &self.0 {
            bar.finish_and_clear();
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Artifact {
    pub path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub artist: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub album: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub release_year: String,
    #[serde(default)]
    pub duration_seconds: f64,
    #[serde(default)]
    pub bandwidth_hz: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bandwidth_rating: String,
    #[serde(default)]
    pub suggested_gain_db: f64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CandidateInfo {
    pub id: String,
    pub title: String,
    pub source: String,
    pub duration_seconds: f64,
    pub score: i32,
    pub url: String,
}
impl From<&Candidate> for CandidateInfo {
    fn from(c: &Candidate) -> Self {
        Self {
            id: c.id.clone(),
            title: c.title.clone(),
            source: c.source.clone(),
            duration_seconds: c.duration,
            score: c.score,
            url: c.url.clone(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub timestamp: DateTime<Utc>,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_steps: Option<u8>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate: Option<CandidateInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Artifact>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}
impl Event {
    pub fn new(kind: &str, phase: &str) -> Self {
        Self {
            timestamp: Utc::now(),
            kind: kind.into(),
            phase: phase.into(),
            step: None,
            total_steps: None,
            message: String::new(),
            percent: None,
            candidate: None,
            result: None,
            error: String::new(),
        }
    }
    pub fn phase(phase: &str, step: u8) -> Self {
        let mut e = Self::new("phase_start", phase);
        e.step = Some(step);
        e.total_steps = Some(5);
        e
    }
}
