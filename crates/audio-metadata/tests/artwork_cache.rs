use audio_metadata::{
    CancellationToken, Error,
    cache::Cache,
    metadata::{Endpoints, Resolver},
    process::Tools,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

#[tokio::test]
async fn artwork_cache_avoids_repeated_downloads_but_honors_cancellation_and_disable() {
    let temp = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/.tmp")).unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let app = axum::Router::new().route(
        "/cover",
        axum::routing::get(move || {
            count.fetch_add(1, Ordering::SeqCst);
            async { b"cover bytes" }
        }),
    );
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/cover", socket.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
    let tools = Tools::new(std::env::var_os("PATH").unwrap(), temp.path().to_owned());
    let cache = Cache::new(temp.path().join("cache"), true);
    let resolver = Resolver::new(
        tools.clone(),
        cache.clone(),
        Endpoints::default(),
        "audio-metadata-tests/0.1.0",
    )
    .unwrap();
    let token = CancellationToken::new();
    assert_eq!(
        resolver.artwork(&url, &token).await.unwrap(),
        b"cover bytes"
    );
    assert_eq!(
        resolver.artwork(&url, &token).await.unwrap(),
        b"cover bytes"
    );
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        resolver.artwork(&url, &cancelled).await,
        Err(Error::Cancelled)
    ));
    let uncached = Resolver::new(
        tools,
        Cache::new(temp.path().join("disabled"), false),
        Endpoints::default(),
        "audio-metadata-tests/0.1.0",
    )
    .unwrap();
    uncached.artwork(&url, &token).await.unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    cache
        .put_bytes("artwork", "expired", b"old", Duration::ZERO)
        .unwrap();
    assert!(cache.get_bytes("artwork", "expired", 10).unwrap().is_none());
    cache
        .put_bytes("artwork", "large", b"123456789", Duration::from_secs(60))
        .unwrap();
    assert!(cache.get_bytes("artwork", "large", 4).unwrap().is_none());
    let disabled = Cache::new(temp.path().join("disabled"), false);
    disabled
        .put_bytes("artwork", "x", b"data", Duration::from_secs(60))
        .unwrap();
    assert!(disabled.get_bytes("artwork", "x", 10).unwrap().is_none());
    assert!(!temp.path().join("disabled").exists());
    server.abort();
}
