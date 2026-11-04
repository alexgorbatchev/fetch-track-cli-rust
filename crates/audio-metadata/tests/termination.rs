//! Process-group termination through real shell fixtures that trap signals.
use audio_metadata::{CancellationToken, Error, process::Tools};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const WAIT: Duration = Duration::from_secs(10);

fn temp() -> tempfile::TempDir {
    tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/.tmp")).unwrap()
}

/// How the fixture's process group responds to SIGTERM.
#[derive(Clone, Copy)]
enum Fixture {
    /// The leader's handler writes more than a pipe buffer to standard error,
    /// a line to standard output, records `cleaned`, and exits.
    NoisyCleanup,
    /// The leader records `term` and exits; its grandchild ignores SIGTERM.
    LeaderExits,
    /// The leader records `term` and keeps running; its grandchild ignores SIGTERM.
    Stubborn,
}

/// Writes the fixture and returns `Tools` that run it as `fixture` with `grace`.
fn tools(dir: &Path, fixture: Fixture, grace: Duration) -> Tools {
    let dir = std::fs::canonicalize(dir).unwrap();
    let ready = format!(
        "echo \"$$ $!\" > \"{dir}/pids.tmp\"\nmv \"{dir}/pids.tmp\" \"{dir}/pids\"\n",
        dir = dir.display()
    );
    let record = dir.join("record");
    let record = record.display();
    let body = match fixture {
        Fixture::NoisyCleanup => format!(
            "trap 'head -c 200000 /dev/zero >&2; echo stopping; echo cleaned >> \"{record}\"; exit 0' TERM\n\
             sleep 30 &\n{ready}while :; do wait; done\n"
        ),
        Fixture::LeaderExits => format!(
            "trap '' TERM\nsleep 30 &\ntrap 'echo term >> \"{record}\"; exit 0' TERM\n\
             {ready}while :; do wait; done\n"
        ),
        Fixture::Stubborn => format!(
            "trap '' TERM\nsleep 30 &\ntrap 'echo term >> \"{record}\"' TERM\n\
             {ready}while :; do wait; done\n"
        ),
    };
    // `sh` reads the script, which avoids the platform's first-execution checks
    // of a just-written executable delaying the fixture past short timeouts.
    std::fs::write(dir.join("fixture.sh"), body).unwrap();
    let mut tools = Tools::new(
        std::env::join_paths([&dir, Path::new("/bin"), Path::new("/usr/bin")]).unwrap(),
        dir,
    );
    tools.termination_grace = grace;
    tools
}

fn root(tools: &Tools) -> PathBuf {
    tools.cwd.clone()
}

/// Waits for the fixture to report its leader and grandchild PIDs.
async fn pids(dir: &Path) -> Vec<i32> {
    let path = dir.join("pids");
    let deadline = Instant::now() + WAIT;
    while !path.exists() {
        assert!(Instant::now() < deadline, "fixture never became ready");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    std::fs::read_to_string(path)
        .unwrap()
        .split_whitespace()
        .map(|pid| pid.parse().unwrap())
        .collect()
}

fn alive(pid: i32) -> bool {
    // SAFETY: kill(2) with signal 0 only checks that the process exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Reports whether `pid` exists and is not a zombie. An exited grandchild stays
/// a zombie until init reaps it, and macOS group signals skip zombies.
fn running(pid: i32) -> bool {
    let output = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let state = String::from_utf8_lossy(&output.stdout);
    !state.trim().is_empty() && !state.trim().starts_with('Z')
}

async fn wait_until_exited(pids: &[i32]) {
    let deadline = Instant::now() + WAIT;
    while pids.iter().any(|pid| alive(*pid)) {
        assert!(Instant::now() < deadline, "processes {pids:?} survived");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn record(tools: &Tools) -> String {
    std::fs::read_to_string(root(tools).join("record")).unwrap_or_default()
}

async fn run(
    tools: &Tools,
    timeout: Duration,
    token: &CancellationToken,
) -> Result<Vec<u8>, Error> {
    tools.run("sh", &["fixture.sh"], 1024, timeout, token).await
}

async fn lines(tools: &Tools, timeout: Duration, token: &CancellationToken) -> Result<(), Error> {
    tools
        .lines::<_, _, Error>("sh", &["fixture.sh"], timeout, token, |_| async { Ok(()) })
        .await
}

/// Cancels `token` once the fixture is ready; returns its PIDs and the cancellation time.
async fn cancel_when_ready(dir: &Path, token: &CancellationToken) -> (Vec<i32>, Instant) {
    let pids = pids(dir).await;
    token.cancel();
    (pids, Instant::now())
}

#[tokio::test]
async fn timeout_sends_sigterm_and_keeps_pipes_open_for_cleanup() {
    let dir = temp();
    let tools = tools(dir.path(), Fixture::NoisyCleanup, WAIT);
    let token = CancellationToken::new();
    let started = Instant::now();
    let (result, pids) = tokio::join!(
        run(&tools, Duration::from_secs(1), &token),
        pids(dir.path()),
    );
    assert!(result.unwrap_err().to_string().contains("timed out"));
    assert_eq!(record(&tools), "cleaned\n", "the SIGTERM handler finished");
    assert!(started.elapsed() < WAIT, "the group exited without SIGKILL");
    for pid in &pids {
        assert!(!running(*pid), "process {pid} outlived the returned run");
    }
}

#[tokio::test]
async fn timeout_sends_sigkill_after_the_grace_period() {
    let dir = temp();
    let grace = Duration::from_millis(300);
    let timeout = Duration::from_secs(1);
    let tools = tools(dir.path(), Fixture::Stubborn, grace);
    let token = CancellationToken::new();
    let started = Instant::now();
    let (result, pids) = tokio::join!(run(&tools, timeout, &token), pids(dir.path()),);
    assert!(result.unwrap_err().to_string().contains("timed out"));
    assert!(
        started.elapsed() >= timeout + grace,
        "SIGKILL waits for the grace period"
    );
    assert_eq!(record(&tools), "term\n", "SIGTERM came first");
    for pid in &pids {
        assert!(!running(*pid), "process {pid} outlived the returned run");
    }
}

#[tokio::test]
async fn line_timeout_sends_sigkill_after_the_grace_period() {
    let dir = temp();
    let grace = Duration::from_millis(300);
    let timeout = Duration::from_secs(1);
    let tools = tools(dir.path(), Fixture::Stubborn, grace);
    let token = CancellationToken::new();
    let started = Instant::now();
    let (result, pids) = tokio::join!(lines(&tools, timeout, &token), pids(dir.path()),);
    assert!(result.unwrap_err().to_string().contains("timed out"));
    assert!(
        started.elapsed() >= timeout + grace,
        "SIGKILL waits for the grace period"
    );
    assert_eq!(record(&tools), "term\n", "SIGTERM came first");
    for pid in &pids {
        assert!(!running(*pid), "process {pid} outlived the returned run");
    }
}

#[tokio::test]
async fn cancellation_sends_sigterm_and_keeps_pipes_open_for_cleanup() {
    let dir = temp();
    let tools = tools(dir.path(), Fixture::NoisyCleanup, WAIT);
    let token = CancellationToken::new();
    let started = Instant::now();
    let (result, (pids, _)) = tokio::join!(
        lines(&tools, WAIT, &token),
        cancel_when_ready(dir.path(), &token),
    );
    assert!(matches!(result, Err(Error::Cancelled)));
    assert_eq!(record(&tools), "cleaned\n", "the SIGTERM handler finished");
    assert!(started.elapsed() < WAIT, "the group exited without SIGKILL");
    for pid in &pids {
        assert!(!running(*pid), "process {pid} outlived the returned run");
    }
}

#[tokio::test]
async fn cancellation_waits_for_a_grandchild_that_ignores_sigterm() {
    let dir = temp();
    let grace = Duration::from_millis(500);
    let tools = tools(dir.path(), Fixture::LeaderExits, grace);
    let token = CancellationToken::new();
    let ((result, finished), (pids, cancelled)) = tokio::join!(
        async { (run(&tools, WAIT, &token).await, Instant::now()) },
        cancel_when_ready(dir.path(), &token),
    );
    assert!(matches!(result, Err(Error::Cancelled)));
    assert_eq!(record(&tools), "term\n");
    assert!(
        finished - cancelled >= grace,
        "returned while the grandchild survived SIGTERM"
    );
    for pid in &pids {
        assert!(!running(*pid), "process {pid} outlived the returned run");
    }
}

#[tokio::test]
async fn dropping_a_run_sends_sigterm_and_keeps_pipes_open_for_cleanup() {
    let dir = temp();
    let tools = tools(dir.path(), Fixture::NoisyCleanup, WAIT);
    let token = CancellationToken::new();
    let pids = tokio::select! {
        result = run(&tools, WAIT, &token) => panic!("fixture finished: {result:?}"),
        pids = pids(dir.path()) => pids,
    };
    wait_until_exited(&pids).await;
    assert_eq!(record(&tools), "cleaned\n", "the SIGTERM handler finished");
}

#[test]
fn runtime_shutdown_waits_for_a_dropped_run_to_be_killed() {
    let dir = temp();
    let grace = Duration::from_millis(300);
    let tools = tools(dir.path(), Fixture::Stubborn, grace);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let pids = runtime.block_on(async {
        let token = CancellationToken::new();
        tokio::select! {
            result = lines(&tools, WAIT, &token) => panic!("fixture finished: {result:?}"),
            pids = pids(dir.path()) => pids,
        }
    });
    let dropped = Instant::now();
    drop(runtime);
    assert!(
        dropped.elapsed() >= grace,
        "shutdown waited through the grace period"
    );
    for pid in &pids {
        assert!(!running(*pid), "process {pid} survived runtime shutdown");
    }
    assert_eq!(record(&tools), "term\n", "SIGTERM came first");
}

#[tokio::test]
async fn completed_runs_terminate_members_left_in_the_group() {
    let dir = temp();
    let tools = Tools::new(std::env::var_os("PATH").unwrap(), dir.path().to_owned());
    let output = tools
        .run(
            "sh",
            &["-c", "sleep 30 >/dev/null 2>&1 & echo $!"],
            64,
            WAIT,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    let member: i32 = String::from_utf8(output).unwrap().trim().parse().unwrap();
    assert!(
        !running(member),
        "the leftover member was terminated before return"
    );
}

#[tokio::test]
async fn cancellation_kills_members_that_flood_their_output_after_the_grace_period() {
    let dir = temp();
    let grace = Duration::from_millis(300);
    let mut tools = Tools::new(std::env::var_os("PATH").unwrap(), dir.path().to_owned());
    tools.termination_grace = grace;
    let token = CancellationToken::new();
    // The flood starts in the SIGTERM handler, when only termination reads the pipes.
    let script = "trap 'trap \"\" TERM; yes & yes & yes & exec yes' TERM\n\
                  echo $$ > pids.tmp; mv pids.tmp pids\n\
                  while :; do sleep 1; done";
    let ((result, finished), (pids, cancelled)) = tokio::join!(
        async {
            let result = tools
                .lines::<_, _, Error>("sh", &["-c", script], WAIT, &token, |_| async { Ok(()) })
                .await;
            (result, Instant::now())
        },
        cancel_when_ready(dir.path(), &token),
    );
    assert!(matches!(result, Err(Error::Cancelled)));
    let elapsed = finished - cancelled;
    assert!(elapsed >= grace, "SIGKILL waits for the grace period");
    assert!(elapsed < WAIT, "draining the flood held off SIGKILL");
    assert!(!running(pids[0]), "the flooding leader is killed");
}

#[test]
fn dropping_a_run_outside_its_runtime_still_terminates_the_group() {
    let dir = temp();
    let grace = Duration::from_millis(300);
    let tools = tools(dir.path(), Fixture::Stubborn, grace);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let token = CancellationToken::new();
    let mut pending = Box::pin(run(&tools, WAIT, &token));
    let pids = runtime.block_on(async {
        tokio::select! {
            result = &mut pending => panic!("fixture finished: {result:?}"),
            pids = pids(dir.path()) => pids,
        }
    });
    drop(runtime);
    drop(pending);
    let deadline = Instant::now() + WAIT;
    while pids.iter().any(|pid| running(*pid)) {
        assert!(Instant::now() < deadline, "processes {pids:?} survived");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(record(&tools), "term\n", "SIGTERM came first");
}
