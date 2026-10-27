use audio_metadata::identity::normalize;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Candidate {
    #[serde(default)]
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub source: String,
    #[serde(default, alias = "webpage_url")]
    pub url: String,
    #[serde(default)]
    pub score: i32,
}
pub fn matches(query: &str, text: &str) -> bool {
    let query = normalize(query);
    let text = normalize(text);
    if text.contains(&query) {
        return true;
    }
    let mut chars = query.chars();
    let mut wanted = chars.next();
    for c in text.chars() {
        if wanted == Some(c) {
            wanted = chars.next();
        }
    }
    wanted.is_none()
}
pub fn clean_title(title: &str) -> String {
    ["(radio edit)", "(edit)", "(short mix)", "(single version)"]
        .iter()
        .fold(title.to_lowercase(), |s, k| s.replace(k, ""))
        .trim()
        .to_owned()
}
pub fn rank(candidates: Vec<Candidate>, artist: &str, title: &str) -> Vec<Candidate> {
    let mut urls = HashSet::new();
    let mut ids = HashSet::new();
    let mut keys = HashSet::new();
    let mut result = Vec::new();
    let title = normalize(&clean_title(title));
    let artist = normalize(artist);
    for mut c in candidates {
        let url = c.url.trim().to_owned();
        let id = (c.source.to_lowercase(), c.id.clone());
        let key = (
            c.source.to_lowercase(),
            normalize(&c.title),
            c.duration as i64,
        );
        if (!url.is_empty() && urls.contains(&url))
            || (!c.id.is_empty() && ids.contains(&id))
            || keys.contains(&key)
        {
            continue;
        }
        if !url.is_empty() {
            urls.insert(url);
        }
        if !c.id.is_empty() {
            ids.insert(id);
        }
        keys.insert(key);
        let text = normalize(&c.title);
        let mut score = if title.is_empty() {
            0
        } else if text.contains(&title) {
            100
        } else if matches(&title, &text) {
            60
        } else {
            -300
        };
        if !artist.is_empty() {
            score += if text.contains(&artist) {
                100
            } else if matches(&artist, &text) {
                50
            } else {
                0
            };
        }
        if [
            "extended",
            "original mix",
            "club mix",
            "dub mix",
            "12 inch",
            "12\"",
            "full version",
        ]
        .iter()
        .any(|s| text.contains(s))
        {
            score += 50;
        }
        if ["radio edit", "short edit", "single edit"]
            .iter()
            .any(|s| text.contains(s))
        {
            score -= 30;
        }
        score += match c.duration {
            d if (240.0..=780.0).contains(&d) => 50 + d as i32,
            d if (150.0..240.0).contains(&d) => 20 + d as i32,
            d if d < 120.0 || d > 900.0 => -300,
            d if d < 150.0 => -60,
            _ => 0,
        };
        c.score = score;
        result.push(c);
    }
    result.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.source.cmp(&b.source))
            .then_with(|| a.url.cmp(&b.url))
    });
    result
}
