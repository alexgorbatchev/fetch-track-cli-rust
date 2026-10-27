# Structured progress

`--progress-target` overrides `--progress-socket`, which overrides `FETCH_TRACK_PROGRESS_TARGET`. A target is `unix://<path>`, `tcp://<host:port>`, `fd://<nonnegative descriptor>`, `stdout`, or `stderr`. `stdout://` and `stderr://` are accepted. Absolute paths, `./`, `../`, and `.sock` paths select Unix sockets; `host:port` selects TCP. Connection timeout is ten seconds.

The caller listens before starting fetch-track. Socket connections are clients; TCP does not add authentication or TLS. Descriptor targets duplicate the inherited descriptor and preserve caller ownership. Each event is one serialized UTF-8 JSON line. Concurrent emitters serialize complete lines; write failures fail the operation. Acquisition stdout contains only NDJSON when stdout is selected.

Every event contains UTC `timestamp` and `type`. Optional fields are `phase`, `step`, `total_steps`, `message`, `percent`, `candidate`, `result`, and `error`. Empty optional strings and absent values are omitted.

| Type | Contract |
| :--- | :--- |
| `phase_start` | `phase`: search/download/verify/metadata; steps 2/3/4/5 with total_steps 5. Skipped phases are absent. |
| `candidate_found` | candidate ID, title, source, duration_seconds, score, url. |
| `candidate_selected` | Exactly one actual selected candidate. |
| `progress` | Download percentage when yt-dlp reports it. |
| `complete` | result always contains path, including skipped optional phases. |
| `error` | error text on acquisition failure, unless already cancelled. |

The result contains `path`, `duration_seconds`, `bandwidth_hz`, `suggested_gain_db`, and available `title`, `artist`, `album`, `release_year`, `bandwidth_rating`, and `status`. Numeric fields can be zero when analysis was skipped. Missing metadata strings are omitted. Paths may begin with `~/`; consumers expand the home prefix before file access.

Warnings about optional phases remain on stderr. A completed acquisition can retain an untagged master with unknown analysis. A complete event establishes the actual saved path rather than provider recognition accuracy.

For inherited descriptors and stdout/stderr progress, pipes and sockets use native nonblocking readiness. Their original flags are restored when the last reporter is dropped, including cancellation and write errors. Reserve the designated descriptor for progress for that lifetime; shared open-file descriptions observe its temporary nonblocking flag. Regular files use native asynchronous file output. A cancelled write may end with an incomplete final line; consumers process only newline-terminated records.
