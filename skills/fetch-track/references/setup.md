# External tools and release channels

Required tools are yt-dlp >=2024.08.01 and FFmpeg/ffprobe >=4.4. Managed executables precede PATH. The managed directory is the platform data directory plus `fetch-track/bin`; XDG variables apply through platform directory resolution.

`dependencies install` installs unsatisfied tools; named installation and update operate on their listed tools. yt-dlp downloads its own upstream executable. FFmpeg and ffprobe use the platform FFmpeg package installer and may request system privileges. Human terminal commands can ask to install unsatisfied tools. Agent and redirected commands use explicit `dependencies install` or `--auto-install`.

The Rust application is unpublished. `self update` reports an unpublished-release error unless all three variables are explicitly configured:

- `FETCH_TRACK_RUST_RELEASE_BASE_URL`: HTTPS base URL; loopback HTTP is accepted for local channels.
- `FETCH_TRACK_RUST_RELEASE_OWNER`: release owner segment.
- `FETCH_TRACK_RUST_RELEASE_REPO`: repository segment.

Configure a channel containing actual Rust artifacts. The release client requests `/<owner>/<repo>/releases/latest`, following the release tag redirect to discover the version, then downloads `/<owner>/<repo>/releases/download/<tag>/<asset>`. Asset names follow the native cli-deps archive convention. See the sibling cli-deps release implementation when preparing a deployment. A version at least as new as the remote version succeeds without replacement. Download and extraction are staged beside the executable before native self-replacement.

Unix assets are `<repository>_<version>_<os>_<arch>.tar.gz`, falling back to `fetch-track_<version>_<os>_<arch>.tar.gz`. The version removes the tag's leading `v`; os is `linux` or `darwin`, and arch maps x86_64 to `amd64` and aarch64 to `arm64`. Archives contain the executable named `fetch-track`.

No release URL is selected by default and no network is needed for `skill`, `--version`, or help. The canonical skill source pointer printed by `skill` does not establish that this local Rust guide has been published.
