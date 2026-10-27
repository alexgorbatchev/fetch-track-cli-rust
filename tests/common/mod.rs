use audio_metadata::{
    cache::Cache,
    metadata::{Endpoints, Resolver},
    process::Tools,
};
use cli_event_stream::Reporter;
use fetch_track_cli::pipeline::Pipeline;
use std::{
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};

pub struct Fixture {
    pub directory: tempfile::TempDir,
    pub bin: PathBuf,
    pub audio: PathBuf,
}
impl Fixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/.tmp")).unwrap();
        let bin = directory.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let audio = directory.path().join("source.flac");
        let ffmpeg = native_tool("ffmpeg");
        let ffprobe = native_tool("ffprobe");
        assert!(
            std::process::Command::new(&ffmpeg)
                .args([
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=14000:sample_rate=48000:duration=8",
                    "-y"
                ])
                .arg(&audio)
                .status()
                .unwrap()
                .success()
        );
        symlink(ffmpeg, bin.join("ffmpeg")).unwrap();
        symlink(ffprobe, bin.join("ffprobe")).unwrap();
        let source = quote(audio.to_str().unwrap());
        let script = format!(
            "#!/bin/sh\nset -eu\nmode=download\npattern=''\nprevious=''\nfor argument in \"$@\"; do\n case \"$argument\" in\n --version) printf '2026.10.01\\n'; exit 0;;\n --dump-single-json) mode=probe;;\n --flat-playlist) mode=search;;\n -x|--audio-format) printf 'unwanted transcoding option\\n' >&2; exit 1;;\n esac\n if [ \"$previous\" = '-o' ]; then pattern=$argument; fi\n previous=$argument\ndone\ncase \"$mode\" in\nprobe) printf '%s\\n' '{{\"title\":\"Artist - Song (Extended Mix)\",\"artist\":\"Artist\",\"duration\":420,\"ext\":\"flac\"}}';;\nsearch) printf '%s\\n' '{{\"id\":\"ABC\",\"title\":\"Artist - Song (Extended Mix)\",\"duration\":420,\"webpage_url\":\"https://example.org/audio\"}}' '{{\"id\":\"short\",\"title\":\"Artist - Song\",\"duration\":30,\"webpage_url\":\"https://example.org/short\"}}';;\ndownload) directory=${{pattern%/*}}; target=\"$directory/Artist - Song [video-id].flac\"; /bin/cp {source} \"$target\"; printf '%s\\n' '__FETCH_PROGRESS__{{\"status\":\"downloading\",\"downloaded_bytes\":25,\"total_bytes\":100}}'; printf '__FETCH_PATH__\"%s\"\\n' \"$target\";;\nesac\n"
        );
        std::fs::write(bin.join("yt-dlp"), script).unwrap();
        std::fs::set_permissions(bin.join("yt-dlp"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        Self {
            directory,
            bin,
            audio,
        }
    }
    pub fn root(&self) -> &Path {
        self.directory.path()
    }
    pub fn pipeline(&self, endpoints: Endpoints, reporter: Reporter) -> Pipeline {
        let tools = Tools::new(self.bin.as_os_str().to_owned(), self.root().to_owned());
        let cache = Cache::new(self.root().join("cache"), true);
        let resolver = Resolver::new(
            tools.clone(),
            cache.clone(),
            endpoints,
            "fetch-track-tests/0.1.0",
        )
        .unwrap();
        Pipeline {
            tools,
            cache,
            resolver,
            reporter,
        }
    }
    pub fn cli(&self) -> std::process::Command {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_fetch-track"));
        command
            .env("PATH", &self.bin)
            .env("XDG_DATA_HOME", self.root().join("data"))
            .env("XDG_CACHE_HOME", self.root().join("cache"))
            .env("AGENT", "1")
            .env_remove("FETCH_TRACK_PROGRESS_TARGET")
            .env_remove("FETCH_TRACK_RUST_RELEASE_BASE_URL")
            .env_remove("FETCH_TRACK_RUST_RELEASE_OWNER")
            .env_remove("FETCH_TRACK_RUST_RELEASE_REPO")
            .current_dir(self.root());
        command
    }
}
fn quote(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "'\\''"))
}

// Some development PATHs contain installation shims rather than the media binaries.
// Resolve their declared executable without invoking an installer or inheriting it
// into the deliberately isolated fixture PATH.
fn native_tool(name: &str) -> PathBuf {
    let found = which::which(name).unwrap();
    if let Ok(script) = std::fs::read_to_string(&found)
        && let Some(path) = script.lines().find_map(|line| {
            line.strip_prefix("TOOL_EXECUTABLE=\"")
                .and_then(|s| s.strip_suffix('"'))
        })
    {
        let path = PathBuf::from(path);
        assert!(path.is_file(), "media shim must already be installed");
        return path;
    }
    found
}
