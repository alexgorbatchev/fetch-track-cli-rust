use audio_metadata::identity::normalize;
use fetch_track_cli::{
    candidate::{Candidate, rank},
    quality::analyze,
    source_url,
};

#[test]
fn ranking_prefers_extended_and_preserves_case_sensitive_ids() {
    let candidates = vec![
        Candidate {
            id: "ABC".into(),
            title: "Björk - Jóga (Radio Edit)".into(),
            duration: 180.0,
            source: "youtube".into(),
            url: "https://youtu.be/ABC".into(),
            ..Default::default()
        },
        Candidate {
            id: "abc".into(),
            title: "Bjork - Joga (Extended Mix)".into(),
            duration: 420.0,
            source: "youtube".into(),
            url: "https://youtu.be/abc".into(),
            ..Default::default()
        },
    ];
    let ranked = rank(candidates, "Björk", "Jóga");
    assert_eq!(ranked.len(), 2);
    assert_eq!(ranked[0].id, "abc");
    assert_eq!(normalize("Ёж Björk"), "еж bjork");
    assert!(source_url("youtube.com/watch?v=ABC").is_some());
    assert!(source_url("notes-youtube.com.txt").is_none());
    assert!(source_url("https://youtube.com.attacker.invalid/x").is_some()); // A genuine HTTP URL remains a generic source.
}

#[test]
fn spectrum_does_not_alias_two_kilohertz_to_fourteen_and_silence_is_unknown() {
    let sine = |hz: f32| {
        (0..48000)
            .map(|n| 0.5 * (std::f32::consts::TAU * hz * n as f32 / 48000.0).sin())
            .collect::<Vec<_>>()
    };
    let low = analyze(&sine(2000.0), 48000).unwrap();
    let high = analyze(&sine(14000.0), 48000).unwrap();
    assert!(low.bandwidth_hz < 4000);
    assert!(high.bandwidth_hz > 13000);
    assert!((low.peak_dbfs + 6.0206).abs() < 0.1);
    assert!((low.rms_dbfs + 9.0309).abs() < 0.1);
    assert_eq!(
        analyze(&vec![0.0; 48000], 48000).unwrap().bandwidth_rating,
        "Insufficient signal"
    );
    assert!(analyze(&[f32::NAN; 16], 48000).is_err());
    assert!(analyze(&[1.0], 48000).is_err());
    assert!(analyze(&[], 48000).is_err());
    assert!(analyze(&[0.5; 16], 0).is_err());
}

#[test]
fn ranking_deduplicates_stably_without_poisoning_later_identities() {
    let candidate = |id: &str, url: &str, title: &str, duration: f64| Candidate {
        id: id.into(),
        url: url.into(),
        title: title.into(),
        duration,
        source: "youtube".into(),
        score: 0,
    };
    let data = vec![
        candidate("one", "url1", "Song", 420.0),
        candidate("two", "url1", "Other", 421.0),
        candidate("two", "url2", "Song variant", 422.0),
        candidate("three", "url3", "Song", 420.0),
        candidate("four", "url4", "S o n g", 170.0),
        candidate("five", "url5", "Unrelated", 80.0),
        candidate("six", "url6", "Song short", 130.0),
        candidate("seven", "url7", "Song long", 800.0),
    ];
    let ranked = rank(data, "", "Song");
    assert_eq!(ranked.len(), 6);
    assert!(ranked.iter().any(|c| c.id == "two" && c.url == "url2"));
    assert!(ranked.iter().all(|c| c.id != "three"));
    assert!(ranked.iter().find(|c| c.id == "four").unwrap().score > 0);
    assert!(!fetch_track_cli::candidate::matches("xyz", "abc"));
    assert!(source_url("/local/file").is_none());
    let samples = |frequency: f32| {
        (0..48000)
            .map(|n| (std::f32::consts::TAU * frequency * n as f32 / 48000.0).sin())
            .collect::<Vec<_>>()
    };
    let wide = analyze(&samples(20000.0), 48000).unwrap();
    assert_eq!(wide.bandwidth_rating, "Wide bandwidth");
    assert!(wide.clipping_warning);
    assert!(wide.suggested_gain_db <= 0.0);
    assert_eq!(
        analyze(&samples(17000.0), 48000).unwrap().bandwidth_rating,
        "Standard"
    );
    assert!(fetch_track_cli::quality::structure("Extended Mix", 190.0).extended);
    assert_eq!(
        fetch_track_cli::quality::structure("Song", 270.0).description,
        "Standard Track (4.0 - 5.0 mins)"
    );
    assert!(fetch_track_cli::quality::structure("Song", 350.0).extended);
}
