use audio_metadata::{
    CancellationToken, Error,
    cache::Cache,
    metadata::{Endpoints, Resolver},
    process::Tools,
};
use axum::{
    Json, Router,
    extract::Query,
    http::StatusCode,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

async fn setup(directory: &Path, app: Router) -> (Resolver, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    let endpoint = |p: &str| reqwest::Url::parse(&format!("{base}{p}")).unwrap();
    let endpoints = Endpoints {
        itunes: endpoint("itunes"),
        musicbrainz: endpoint("mb"),
        covers: endpoint("covers/"),
        acoustid: endpoint("acoustid"),
        shazam: shazam_client::Endpoints {
            recognition: Some(endpoint("shazam")),
            ..Default::default()
        },
    };
    let tools = Tools::new(std::env::var_os("PATH").unwrap(), directory.to_owned());
    let resolver = Resolver::new(
        tools,
        Cache::new(directory.join("cache"), true),
        endpoints,
        "audio-metadata-tests/0.1.0",
    )
    .unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (resolver, server)
}
fn temp() -> tempfile::TempDir {
    tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/.tmp")).unwrap()
}
fn audio(directory: &Path) -> std::path::PathBuf {
    let path = directory.join("track.wav");
    let output = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=500:sample_rate=16000:duration=8",
            "-y",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(output.status.success());
    path
}
fn itunes() -> Value {
    json!({"results":[{"trackName":"Song","artistName":"Wrong Artist"},{"trackName":"Song","artistName":"Artist","collectionName":"Album","primaryGenreName":"House","releaseDate":"2001-02-03T12:00:00Z","artworkUrl100":"https://example.org/100x100bb.jpg"}]})
}

#[tokio::test]
async fn itunes_matches_artist_and_title_dates_and_artwork_and_http_errors() {
    let temp = temp();
    let seen = Arc::new(Mutex::new(HashMap::new()));
    let saved = seen.clone();
    let app = Router::new()
        .route(
            "/itunes",
            get(move |Query(query): Query<HashMap<String, String>>| {
                *saved.lock().unwrap() = query;
                async { Json(itunes()) }
            }),
        )
        .route(
            "/art",
            get(|| async {
                (
                    [(axum::http::header::CONTENT_TYPE, "image/png")],
                    b"image bytes",
                )
            }),
        );
    let (mut r, server) = setup(temp.path(), app).await;
    let token = CancellationToken::new();
    let result = r
        .itunes("Artist Song", "Artist", "Song", &token)
        .await
        .unwrap();
    assert_eq!(
        (
            result.artist.as_str(),
            result.title.as_str(),
            result.release_date.as_str()
        ),
        ("Artist", "Song", "2001-02-03")
    );
    assert_eq!(result.release_year(), "2001");
    assert!(result.cover_url.ends_with("1400x1400bb.jpg"));
    assert_eq!(seen.lock().unwrap()["term"], "Artist Song");
    assert!(r.itunes("x", "Nobody", "Song", &token).await.is_err());
    assert!(r.itunes("x", "Artist", "", &token).await.is_err());
    let art = r.endpoints.itunes.join("art").unwrap();
    assert_eq!(
        r.artwork(art.as_str(), &token).await.unwrap(),
        b"image bytes"
    );
    assert!(r.artwork("file:///private", &token).await.is_err());
    assert!(r.artwork("not a URL", &token).await.is_err());
    r.endpoints.itunes = r.endpoints.itunes.join("absent").unwrap();
    assert!(r.itunes("x", "Artist", "Song", &token).await.is_err());
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        r.itunes("x", "a", "b", &cancelled).await,
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        r.artwork(art.as_str(), &cancelled).await,
        Err(Error::Cancelled)
    ));
    server.abort();
}

#[tokio::test]
async fn musicbrainz_respects_rate_limit_and_front_approved_release_cover() {
    let temp = temp();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    let app=Router::new().route("/mb",get(move||{seen.lock().unwrap().push(tokio::time::Instant::now());async{Json(json!({"recordings":[{"title":"Wrong"},{"title":"Song","artist-credit":[{"name":"Artist","joinphrase":" & "},{"name":"Guest"}],"releases":[{"id":"release-id","title":"Album","date":"1998-06"}]}]}))}})).route("/covers/release/release-id",get(||async{Json(json!({"images":[{"front":false,"image":"back"},{"front":true,"approved":false,"image":"unapproved"},{"front":true,"approved":true,"image":"front"}]}))}));
    let (r, server) = setup(temp.path(), app).await;
    let token = CancellationToken::new();
    let first = r
        .musicbrainz("Song Artist", "Artist", "Song", &token)
        .await
        .unwrap();
    assert_eq!(first.cover_url, "front");
    assert_eq!(first.artist, "Artist & Guest");
    assert_eq!(first.release_date, "1998-06");
    r.musicbrainz("Song Artist", "Artist", "Song", &token)
        .await
        .unwrap();
    let times = requests.lock().unwrap().clone();
    assert!(times[1].duration_since(times[0]) >= Duration::from_millis(990));
    assert!(r.musicbrainz("x", "Artist", "Other", &token).await.is_err());
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        r.musicbrainz("x", "Artist", "Song", &cancelled).await,
        Err(Error::Cancelled)
    ));
    server.abort();
}

#[tokio::test]
async fn acoustic_chain_recognition_enrichment_content_cache_and_cancel() {
    let temp = temp();
    let file = audio(temp.path());
    let count = Arc::new(Mutex::new(0));
    let recognized = count.clone();
    let app=Router::new().route("/shazam",post(move|Json(body):Json<Value>|{assert!(body["signature"]["uri"].as_str().unwrap().starts_with("data:audio/vnd.shazam.sig;base64,"));*recognized.lock().unwrap()+=1;async{Json(json!({"track":{"title":"Song","subtitle":"Artist","genres":{"primary":"House"},"sections":[{"metadata":[{"title":"Album","text":"Recognized Album"},{"title":"Released","text":"2000"}]}]}}))}})).route("/itunes",get(||async{Json(itunes())}));
    let (r, server) = setup(temp.path(), app).await;
    let token = CancellationToken::new();
    let result = r
        .resolve(&file, "Artist Song", "Artist", "Song", &token)
        .await
        .unwrap();
    assert_eq!(result.provider, "Shazam");
    assert_eq!(result.album, "Recognized Album");
    assert_eq!(result.release_date, "2000");
    assert!(result.cover_url.contains("1400x1400"));
    let second = r
        .resolve(&file, "Artist Song", "Artist", "Song", &token)
        .await
        .unwrap();
    assert_eq!(second.title, "Song");
    assert_eq!(*count.lock().unwrap(), 1);
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        r.resolve(&file, "different", "Artist", "Song", &cancelled)
            .await,
        Err(Error::Cancelled)
    ));
    server.abort();
}

#[tokio::test]
async fn acoustid_selects_highest_confidence_and_correct_release_cover_kind() {
    let temp = temp();
    let file = audio(temp.path());
    let form = Arc::new(Mutex::new(HashMap::new()));
    let saved = form.clone();
    let app=Router::new().route("/acoustid",post(move|axum::Form(fields):axum::Form<HashMap<String,String>>|{*saved.lock().unwrap()=fields;async{Json(json!({"status":"ok","results":[{"score":0.71,"recordings":[{"title":"Weaker"}]},{"score":0.99,"recordings":[{"title":"Song","artists":[{"name":"Artist"}],"releases":[{"id":"release-id","title":"Album","date":{"year":1999,"month":12,"day":31}}],"releasegroups":[{"id":"group-id","title":"Group"}]}]}]}))}})).route("/covers/release/release-id",get(||async{Json(json!({"images":[{"front":true,"approved":true,"image":"release-cover"}]}))}));
    let (r, server) = setup(temp.path(), app).await;
    let token = CancellationToken::new();
    // Recognition must derive duration from decoded samples, even when a caller
    // has no prior probe (e.g. fetch-track --skip-verify).
    let result = r.acoustid(&file, &token).await.unwrap();
    assert_eq!(result.title, "Song");
    assert_eq!(result.release_date, "1999-12-31");
    assert_eq!(result.cover_url, "release-cover");
    let fields = form.lock().unwrap().clone();
    assert_eq!(fields["duration"], "8");
    assert!(!fields["fingerprint"].is_empty());
    server.abort();
}

#[tokio::test]
async fn all_remote_failures_fall_back_without_fabricated_year_or_provider() {
    let temp = temp();
    let file = audio(temp.path());
    let app = Router::new()
        .route("/shazam", post(|| async { Json(json!({"matches":[]})) }))
        .route(
            "/acoustid",
            post(|| async { Json(json!({"status":"error","error":{"message":"lookup failed"}})) }),
        )
        .route("/itunes", get(|| async { StatusCode::NOT_FOUND }))
        .route("/mb", get(|| async { Json(json!({"recordings":[]})) }));
    let (r, server) = setup(temp.path(), app).await;
    let result = r
        .resolve(
            &file,
            "Artist Song",
            "Artist",
            "Song",
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(result.provider, "Raw fallback");
    assert!(result.release_date.is_empty());
    assert_eq!(result.artist, "Artist");
    server.abort();
}

#[tokio::test]
async fn acoustid_cache_shares_recognition_across_lossless_encodings() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let temp = temp();
    let wav = audio(temp.path());
    let flac = temp.path().join("same-audio.flac");
    assert!(
        std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&wav)
            .arg(&flac)
            .status()
            .unwrap()
            .success()
    );
    assert_ne!(
        Cache::file_key(&wav).unwrap(),
        Cache::file_key(&flac).unwrap()
    );
    let requests = Arc::new(AtomicUsize::new(0));
    let count = requests.clone();
    let app = Router::new().route("/acoustid", post(move || {
        count.fetch_add(1, Ordering::SeqCst);
        async { Json(json!({"status":"ok","results":[{"score":0.99,"recordings":[{"title":"Song","artists":[{"name":"Artist"}]}]}]})) }
    }));
    let (r, server) = setup(temp.path(), app).await;
    let token = CancellationToken::new();
    let first = r.acoustid(&wav, &token).await.unwrap();
    let second = r.acoustid(&flac, &token).await.unwrap();
    assert_eq!(second.title, first.title);
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        r.acoustid(&flac, &cancelled).await,
        Err(Error::Cancelled)
    ));
    server.abort();
}

#[tokio::test]
async fn shazam_selects_alternate_artwork_and_upgrades_native_image_sizes() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let temp = temp();
    let file = audio(temp.path());
    let fixtures = vec![
        (
            json!({"images":{"coverarthq":"https://example.org/400x400cc.jpg","background":"wrong"}}),
            "https://example.org/1400x1400cc.jpg",
        ),
        (
            json!({"images":{"coverart":"https://example.org/800x800cc.jpg"}}),
            "https://example.org/1400x1400cc.jpg",
        ),
        (
            json!({"images":{"background":"https://example.org/background.jpg"}}),
            "https://example.org/background.jpg",
        ),
        (
            json!({"share":{"image":"https://example.org/share.jpg","avatar":"wrong"}}),
            "https://example.org/share.jpg",
        ),
        (
            json!({"share":{"avatar":"https://example.org/avatar.jpg"}}),
            "https://example.org/avatar.jpg",
        ),
        (
            json!({"hub":{"image":"https://example.org/hub.jpg"}}),
            "https://example.org/hub.jpg",
        ),
        (
            json!({"hub":{"options":[{}, {"image":"https://example.org/option.jpg"}]}}),
            "https://example.org/option.jpg",
        ),
        (
            json!({"sections":[{}, {"avatar":"https://example.org/section.jpg"}]}),
            "https://example.org/section.jpg",
        ),
        (
            json!({"sections":[{"metapages":[{}, {"image":"https://example.org/page.jpg"}]}]}),
            "https://example.org/page.jpg",
        ),
    ];
    let tracks: Vec<_> = fixtures
        .iter()
        .map(|(fields, _)| {
            let mut track = fields.clone();
            track["title"] = "Song".into();
            track["subtitle"] = "Artist".into();
            track
        })
        .collect();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let app = Router::new().route(
        "/shazam",
        post(move || {
            let track = tracks[count.fetch_add(1, Ordering::SeqCst)].clone();
            async move { Json(json!({"track":track})) }
        }),
    );
    let (r, server) = setup(temp.path(), app).await;
    for (_, expected) in fixtures {
        let result = r.shazam(&file, &CancellationToken::new()).await.unwrap();
        assert_eq!(result.cover_url, expected);
        assert!(result.album.is_empty());
        assert!(result.genre.is_empty());
    }
    assert_eq!(calls.load(Ordering::SeqCst), 9);
    server.abort();
}
