use crate::{
    CancellationToken, Error, Result,
    process::{Tools, args},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Inspection {
    pub path: PathBuf,
    pub filename: String,
    pub format: String,
    pub codec: String,
    pub duration_seconds: f64,
    pub bit_rate_kbps: u32,
    pub sample_rate_hz: u32,
    pub channels: u32,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub date: String,
    pub year: Option<u32>,
    pub bpm: Option<f64>,
    pub genre: String,
    pub comment: String,
    pub label: String,
    pub composer: String,
    pub remixer: String,
    pub has_artwork: bool,
    pub artwork_width: u32,
    pub artwork_height: u32,
    pub is_square_artwork: bool,
    pub tags: BTreeMap<String, String>,
}

fn number(value: &Value) -> f64 {
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|v| v.is_finite() && *v >= 0.0)
        .unwrap_or(0.0)
}
fn merge(tags: &mut BTreeMap<String, String>, source: &Value) {
    if let Some(object) = source.as_object() {
        for (key, value) in object {
            if let Some(value) = value.as_str().map(str::trim).filter(|v| !v.is_empty()) {
                tags.entry(key.to_lowercase())
                    .or_insert_with(|| value.to_owned());
            }
        }
    }
}
fn tag(tags: &BTreeMap<String, String>, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| tags.get(*key))
        .cloned()
        .unwrap_or_default()
}

/// Inspect a local audio file. Container tags take precedence over audio-stream tags.
pub async fn inspect(tools: &Tools, path: &Path, token: &CancellationToken) -> Result<Inspection> {
    crate::check_cancel(token)?;
    std::fs::metadata(path)?;
    let mut command: Vec<std::ffi::OsString> = args(&[
        "-v",
        "error",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
    ])
    .into_iter()
    .map(Into::into)
    .collect();
    command.push(path.as_os_str().to_owned());
    let bytes = tools
        .run(
            "ffprobe",
            &command,
            8 * 1024 * 1024,
            Duration::from_secs(30),
            token,
        )
        .await?;
    let data: Value = serde_json::from_slice(&bytes)?;
    let streams = data["streams"]
        .as_array()
        .ok_or_else(|| Error::Metadata("ffprobe did not return streams".into()))?;
    let audio = streams
        .iter()
        .find(|v| v["codec_type"] == "audio")
        .ok_or_else(|| Error::Invalid("file has no audio stream".into()))?;
    let mut tags = BTreeMap::new();
    merge(&mut tags, &data["format"]["tags"]);
    merge(&mut tags, &audio["tags"]);
    let date = tag(&tags, &["date", "year", "tdrc", "tyer"]);
    let mut info = Inspection {
        path: path.to_owned(),
        filename: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        format: data["format"]["format_name"]
            .as_str()
            .unwrap_or_default()
            .into(),
        codec: audio["codec_name"].as_str().unwrap_or_default().into(),
        duration_seconds: number(&data["format"]["duration"]).max(number(&audio["duration"])),
        bit_rate_kbps: (number(&data["format"]["bit_rate"]).max(number(&audio["bit_rate"]))
            / 1000.0) as u32,
        sample_rate_hz: number(&audio["sample_rate"]) as u32,
        channels: number(&audio["channels"]) as u32,
        title: tag(&tags, &["title", "tit2"]),
        artist: tag(&tags, &["artist", "tpe1", "album_artist", "tpe2"]),
        album: tag(&tags, &["album", "talb"]),
        year: date.get(..4).and_then(|v| v.parse().ok()),
        date,
        bpm: tag(&tags, &["bpm", "tbpm"])
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && *v > 0.0),
        genre: tag(&tags, &["genre", "tcon"]),
        comment: tag(&tags, &["comment", "comm", "description"]),
        label: tag(&tags, &["label", "publisher", "tpub"]),
        composer: tag(&tags, &["composer", "tcom"]),
        remixer: tag(&tags, &["remixer", "remixed_by", "tpe4"]),
        tags,
        ..Default::default()
    };
    if let Some(stream) = streams
        .iter()
        .find(|v| v["disposition"]["attached_pic"] == 1)
    {
        info.has_artwork = true;
        info.artwork_width = number(&stream["width"]) as u32;
        info.artwork_height = number(&stream["height"]) as u32;
    }
    // Vorbis/Opus artwork is native tag data, not necessarily an ffprobe video stream.
    use lofty::file::TaggedFileExt;
    if let Ok(file) = lofty::read_from_path(path) {
        let picture = file
            .tags()
            .iter()
            .flat_map(|tag| tag.pictures())
            .find(|p| p.pic_type() == lofty::picture::PictureType::CoverFront);
        if let Some(picture) = picture {
            info.has_artwork = true;
            if picture.data().len() <= 20 * 1024 * 1024 {
                use image::ImageReader;
                if let Ok(reader) =
                    ImageReader::new(std::io::Cursor::new(picture.data())).with_guessed_format()
                    && let Ok((width, height)) = reader.into_dimensions()
                {
                    info.artwork_width = width;
                    info.artwork_height = height;
                }
            }
        }
    }
    info.is_square_artwork = info.artwork_width > 0 && info.artwork_width == info.artwork_height;
    crate::check_cancel(token)?;
    Ok(info)
}
