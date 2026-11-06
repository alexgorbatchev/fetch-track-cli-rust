#![cfg(target_os = "linux")]

use audio_metadata::{CancellationToken, Error, process::Tools};
use std::{
    io,
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const WAIT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(10);

// This integration-test binary owns all its children. Linux subreaping lets it
// retain an exited grandchild in the process group until the test releases it,
// exercising the actual post-SIGKILL wait without mocking process operations.
#[test]
fn cleanup_retains_ownership_until_the_killed_group_is_reaped()
-> Result<(), Box<dyn std::error::Error>> {
    let _subreaper = Subreaper::new()?;
    for (lines, cancel) in [(false, true), (false, false), (true, true), (true, false)] {
        let mut fixture = Fixture::new()?;
        let path = fixture.dir.as_ref().unwrap().path().join("pids");
        let task =
            fixture
                .runtime
                .as_ref()
                .unwrap()
                .spawn(command(path, fixture.token.clone(), lines));
        fixture.wait_ready()?;
        if cancel {
            fixture.token.cancel();
        } else {
            task.abort();
        }
        fixture.wait_for_zombie()?;

        let runtime = fixture.runtime.take().unwrap();
        let (finished, receiver) = mpsc::channel();
        fixture.shutdown = Some(thread::spawn(move || {
            let result = runtime.block_on(task);
            drop(runtime);
            let _ = finished.send(result); // The fixture may already be unwinding.
        }));
        let result = fixture.observe_completion(receiver)?;
        if cancel {
            assert!(matches!(result, Ok(Err(Error::Cancelled))));
        } else {
            assert!(result.unwrap_err().is_cancelled());
        }
    }

    // A handle can outlive its runtime. Entering it makes spawn_blocking reject
    // the cleanup task, exercising Reaper's destructor instead of its run body.
    let mut fixture = Fixture::new()?;
    let path = fixture.dir.as_ref().unwrap().path().join("pids");
    let mut run = Box::pin(command(path.clone(), fixture.token.clone(), false));
    fixture.runtime.as_ref().unwrap().block_on(async {
        tokio::select! {
            result = &mut run => panic!("command finished before cancellation: {result:?}"),
            ready = tokio::time::timeout(WAIT, async {
                while std::fs::read_to_string(&path).unwrap_or_default().lines().count() != 2 {
                    tokio::time::sleep(POLL).await;
                }
            }) => ready,
        }
    })?;
    fixture.wait_ready()?;
    let runtime = fixture.runtime.take().unwrap();
    let handle = runtime.handle().clone();
    let (finished, receiver) = mpsc::channel();
    fixture.shutdown = Some(thread::spawn(move || {
        drop(runtime);
        let _entered = handle.enter();
        drop(run);
        let _ = finished.send(());
    }));
    fixture.wait_for_zombie()?;
    fixture.observe_completion(receiver)?;
    Ok(())
}

async fn command(
    path: PathBuf,
    token: CancellationToken,
    lines: bool,
) -> audio_metadata::Result<()> {
    let mut tools = Tools::new("/bin:/usr/bin".into(), path.parent().unwrap().into());
    tools.termination_grace = Duration::from_millis(50);
    let args = [
        std::ffi::OsStr::new("-c"),
        std::ffi::OsStr::new(
            "set -e; echo $$ > \"$1\"; trap '' TERM; sleep 30 & echo $! >> \"$1\"; wait",
        ),
        std::ffi::OsStr::new("sh"),
        path.as_os_str(),
    ];
    if lines {
        tools
            .lines("sh", &args, WAIT, &token, |_| async { Ok(()) })
            .await
    } else {
        tools.run("sh", &args, 1024, WAIT, &token).await.map(drop)
    }
}

struct Subreaper;

impl Subreaper {
    fn new() -> io::Result<Self> {
        // SAFETY: this process-local prctl option takes an integer, not a pointer.
        if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self)
    }
}

impl Drop for Subreaper {
    fn drop(&mut self) {
        // SAFETY: restore the process-local flag after all fixtures have dropped.
        assert_eq!(
            unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 0, 0, 0, 0) },
            0
        );
    }
}

struct Fixture {
    dir: Option<tempfile::TempDir>,
    runtime: Option<tokio::runtime::Runtime>,
    token: CancellationToken,
    group: Option<libc::pid_t>,
    shutdown: Option<thread::JoinHandle<()>>,
    finished: bool,
}

impl Fixture {
    fn new() -> io::Result<Self> {
        Ok(Self {
            dir: Some(tempfile::tempdir()?),
            runtime: Some(tokio::runtime::Runtime::new()?),
            token: CancellationToken::new(),
            group: None,
            shutdown: None,
            finished: false,
        })
    }

    fn observe_completion<T>(
        &mut self,
        receiver: mpsc::Receiver<T>,
    ) -> Result<T, Box<dyn std::error::Error>> {
        // The old implementation abandoned this still-populated group after a
        // one-second final wait. The channel observes completion, not scheduling.
        let premature = match receiver.recv_timeout(Duration::from_millis(1500)) {
            Ok(result) => Some(result),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(error) => return Err(error.into()),
        };
        self.reap_members()?;
        let completed_early = premature.is_some();
        let result = match premature {
            Some(result) => result,
            None => receiver.recv_timeout(WAIT)?,
        };
        self.finish()?;
        assert!(!completed_early, "cleanup completed with an unreaped group");
        Ok(result)
    }

    fn pids(&mut self) -> io::Result<Vec<libc::pid_t>> {
        let path = self.dir.as_ref().unwrap().path().join("pids");
        let contents = match std::fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let pids = contents
            .split_whitespace()
            .map(|value| {
                value
                    .parse()
                    .ok()
                    .filter(|pid| *pid > 0)
                    .ok_or_else(|| io::Error::other("invalid fixture PID"))
            })
            .collect::<io::Result<Vec<_>>>()?;
        self.group = pids.first().copied();
        Ok(pids)
    }

    fn wait_ready(&mut self) -> io::Result<()> {
        let deadline = Instant::now() + WAIT;
        while self.pids()?.len() != 2 {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "fixture did not start",
                ));
            }
            thread::sleep(POLL);
        }
        Ok(())
    }

    fn wait_for_zombie(&mut self) -> io::Result<()> {
        let member = self.pids()?[1];
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some((state, parent, _)) = process_state(member)?
                && state == "Z"
                && parent == std::process::id() as i32
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "member did not exit",
                ));
            }
            thread::sleep(POLL);
        }
    }

    fn reap_members(&self) -> io::Result<()> {
        let Some(group) = self.group else {
            return Ok(());
        };
        // On failure the shell could be killed between spawning and recording
        // its member. Discover only this fixture's adopted group members, and
        // leave the leader to Tokio's Child handle.
        for entry in std::fs::read_dir("/proc")? {
            let entry = entry?;
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.parse().ok())
                .filter(|pid| *pid > 0)
            else {
                continue;
            };
            if pid == group
                && !(self.runtime.is_none()
                    && self
                        .shutdown
                        .as_ref()
                        .is_some_and(|task| task.is_finished()))
            {
                continue;
            }
            if let Some((state, parent, member_group)) = process_state(pid)?
                && state == "Z"
                && parent == std::process::id() as i32
                && member_group == group
            {
                // SAFETY: only this fixture's adopted children are selected;
                // a null status pointer discards the already-observed exit status.
                if unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) } == -1 {
                    return Err(io::Error::last_os_error());
                }
            }
        }
        Ok(())
    }

    fn finish(&mut self) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }
        self.token.cancel();
        self.pids()?;
        if let Some(group) = self.group {
            signal(group, libc::SIGTERM)?;
            let kill_at = Instant::now() + Duration::from_millis(200);
            let deadline = Instant::now() + WAIT;
            loop {
                self.reap_members()?;
                if !signal(group, 0)? {
                    break;
                }
                if Instant::now() >= kill_at {
                    signal(group, libc::SIGKILL)?;
                }
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "fixture group survived",
                    ));
                }
                thread::sleep(POLL);
            }
        }
        drop(self.runtime.take());
        if let Some(shutdown) = self.shutdown.take() {
            shutdown
                .join()
                .map_err(|_| io::Error::other("runtime shutdown panicked"))?;
        }
        self.finished = true;
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            if let Some(dir) = self.dir.take() {
                eprintln!(
                    "fixture cleanup failed: {error}; retained {}",
                    dir.keep().display()
                );
            }
            if let Some(runtime) = self.runtime.take() {
                runtime.shutdown_timeout(WAIT);
            }
        }
    }
}

fn signal(group: libc::pid_t, signal: libc::c_int) -> io::Result<bool> {
    // SAFETY: group is a positive fixture-owned PID; kill has no pointer arguments.
    if unsafe { libc::kill(-group, signal) } == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(false)
    } else {
        Err(error)
    }
}

fn process_state(pid: libc::pid_t) -> io::Result<Option<(String, i32, i32)>> {
    let path = PathBuf::from(format!("/proc/{pid}/stat"));
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let (_, fields) = contents
        .rsplit_once(") ")
        .ok_or_else(|| io::Error::other("invalid process stat"))?;
    let mut fields = fields.split_whitespace();
    let state = fields
        .next()
        .ok_or_else(|| io::Error::other("missing process state"))?;
    let parent = fields
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| io::Error::other("missing parent PID"))?;
    let group = fields
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| io::Error::other("missing group PID"))?;
    Ok(Some((state.into(), parent, group)))
}
