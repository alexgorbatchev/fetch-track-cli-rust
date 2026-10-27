use crate::{CancellationToken, Error, Result, metadata::Metadata};
use lofty::{
    config::WriteOptions,
    file::TaggedFileExt,
    picture::{Picture, PictureType},
    tag::{Accessor, ItemKey, Tag, TagExt},
};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
use unicode_normalization::UnicodeNormalization;

pub fn clean_filename(raw: &str) -> String {
    let mut bracket = 0;
    let mut result = String::new();
    for c in raw.nfc() {
        match c {
            '[' => bracket += 1,
            ']' => bracket = (bracket - 1).max(0),
            _ if bracket > 0 => {}
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => result.push(' '),
            c if c.is_control() => {}
            _ => result.push(c),
        }
    }
    let result = result
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches([' ', '.'])
        .to_owned();
    if result.is_empty() {
        "Unknown Track".into()
    } else {
        result
    }
}

pub fn square_art(bytes: &[u8]) -> Result<Vec<u8>> {
    use image::{ImageReader, Limits, imageops::FilterType};
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| Error::Metadata(e.to_string()))?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(16000);
    limits.max_image_height = Some(16000);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|e| Error::Metadata(e.to_string()))?
        .resize_to_fill(1400, 1400, FilterType::Lanczos3);
    let mut out = Cursor::new(Vec::new());
    image
        .write_to(&mut out, image::ImageFormat::Jpeg)
        .map_err(|e| Error::Metadata(e.to_string()))?;
    Ok(out.into_inner())
}

/// A partial update: absent fields retain their existing native tags.
#[derive(Clone, Debug, Default)]
pub struct TagUpdate {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub date: Option<String>,
    pub comment: Option<String>,
    pub artwork: Option<Vec<u8>>,
}

/// Atomically write a tagged copy, rejecting an unrelated existing destination.
pub fn write(
    source: &Path,
    target: &Path,
    update: &TagUpdate,
    token: &CancellationToken,
) -> Result<PathBuf> {
    crate::check_cancel(token)?;
    let out_dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(out_dir)?;
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .ok_or_else(|| Error::Invalid("audio file has no extension".into()))?;
    let same = source == target
        || (target.exists() && std::fs::canonicalize(source)? == std::fs::canonicalize(target)?);
    if target.exists() && !same {
        return Err(Error::Invalid(format!(
            "destination already exists: {}",
            target.display()
        )));
    }
    let mut staged = tempfile::Builder::new()
        .prefix(".audio-metadata-")
        .suffix(&format!(".{ext}"))
        .tempfile_in(out_dir)?;
    let mut input = std::fs::File::open(source)?;
    use std::io::{Read, Write};
    let mut buffer = [0; 65536];
    loop {
        crate::check_cancel(token)?;
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        staged.write_all(&buffer[..count])?;
    }
    let mut file =
        lofty::read_from_path(staged.path()).map_err(|e| Error::Metadata(e.to_string()))?;
    let tag_type = file.primary_tag_type();
    if file.primary_tag().is_none() {
        file.insert_tag(Tag::new(tag_type));
    }
    let tag = file.primary_tag_mut().expect("inserted primary tag");
    if let Some(v) = &update.title {
        tag.set_title(v.clone());
    }
    if let Some(v) = &update.artist {
        tag.set_artist(v.clone());
    }
    if let Some(v) = &update.album {
        tag.set_album(v.clone());
    }
    if let Some(v) = &update.genre {
        tag.set_genre(v.clone());
    }
    if let Some(v) = &update.date {
        tag.insert_text(ItemKey::RecordingDate, v.clone());
    }
    if let Some(v) = &update.comment {
        tag.set_comment(v.clone());
    }
    if let Some(bytes) = &update.artwork {
        let bytes = square_art(bytes)?;
        let mut picture = Picture::from_reader(&mut Cursor::new(bytes))
            .map_err(|e| Error::Metadata(e.to_string()))?;
        picture.set_pic_type(PictureType::CoverFront);
        tag.remove_picture_type(PictureType::CoverFront);
        tag.push_picture(picture);
    }
    tag.save_to_path(staged.path(), WriteOptions::default().use_id3v23(true))
        .map_err(|e| Error::Metadata(e.to_string()))?;
    staged.as_file().sync_all()?;
    crate::check_cancel(token)?;
    if same {
        staged.persist(target).map_err(|e| e.error)?;
    } else {
        staged.persist_noclobber(target).map_err(|e| e.error)?;
    }
    Ok(target.to_owned())
}

/// Stage a complete tagged copy next to its destination. Source audio remains intact on failure.
pub fn apply(
    source: &Path,
    metadata: &Metadata,
    out_dir: &Path,
    art: Option<&[u8]>,
    token: &CancellationToken,
) -> Result<PathBuf> {
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .ok_or_else(|| Error::Invalid("audio file has no extension".into()))?;
    let stem = if metadata.artist.is_empty() {
        metadata.title.clone()
    } else {
        format!("{} - {}", metadata.artist, metadata.title)
    };
    let target = out_dir.join(format!("{}.{ext}", clean_filename(&stem)));
    let update = TagUpdate {
        title: Some(metadata.title.clone()),
        artist: Some(metadata.artist.clone()),
        album: (!metadata.album.is_empty()).then(|| metadata.album.clone()),
        genre: (!metadata.genre.is_empty()).then(|| metadata.genre.clone()),
        date: (!metadata.release_date.is_empty()).then(|| metadata.release_date.clone()),
        comment: Some(format!(
            "Source: {} | Metadata: {} | Fetched: {}",
            metadata.audio_source_url,
            metadata.provider,
            metadata.fetched_at.format("%Y-%m-%d")
        )),
        artwork: art.map(<[u8]>::to_vec),
    };
    write(source, &target, &update, token)
}

pub fn clean_output(source: &Path, out_dir: &Path) -> Result<PathBuf> {
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown Track");
    let ext = source
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("audio");
    let target = out_dir.join(format!("{}.{ext}", clean_filename(stem)));
    if source == target {
        return Ok(target);
    }
    let mut staged = tempfile::NamedTempFile::new_in(out_dir)?;
    std::io::copy(&mut std::fs::File::open(source)?, &mut staged)?;
    staged.as_file().sync_all()?;
    staged.persist_noclobber(&target).map_err(|e| e.error)?;
    Ok(target)
}
