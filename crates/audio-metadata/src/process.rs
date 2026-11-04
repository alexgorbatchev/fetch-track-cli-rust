use crate::{CancellationToken, Error, Result};
use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{
    io::AsyncReadExt,
    process::{Child, ChildStderr, ChildStdout, Command},
};

/// Time a process group gets to exit after SIGTERM before it receives SIGKILL.
pub const DEFAULT_TERMINATION_GRACE: Duration = cli_deps::DEFAULT_TERMINATION_GRACE;

/// Runs external programs found on `path`, each in its own process group.
///
/// A program that times out, is cancelled, fails its output limits, or whose
/// `run`/`lines` future is dropped is ended with its whole process group:
/// SIGTERM first, then SIGKILL for members still alive after
/// `termination_grace`. The group gets that signal sequence even after a
/// successful exit when descendants remain. Programs such as PyInstaller
/// one-file builds remove their temporary files only when they can handle
/// SIGTERM.
///
/// Children do not share the caller's foreground process group, so terminal
/// signals such as Ctrl-C do not reach them; translate SIGINT and SIGTERM into
/// token cancellation.
#[derive(Clone, Debug)]
pub struct Tools {
    pub path: OsString,
    pub cwd: PathBuf,
    pub termination_grace: Duration,
}
impl Tools {
    /// Uses `DEFAULT_TERMINATION_GRACE`.
    pub fn new(path: OsString, cwd: PathBuf) -> Self {
        Self {
            path,
            cwd,
            termination_grace: DEFAULT_TERMINATION_GRACE,
        }
    }
    pub fn program(&self, name: &str) -> Result<PathBuf> {
        which::which_in(name, Some(&self.path), &self.cwd)
            .map_err(|e| Error::Process(format!("{name}: {e}")))
    }
    pub async fn run(
        &self,
        name: &str,
        args: &[impl AsRef<OsStr>],
        limit: usize,
        timeout: Duration,
        token: &CancellationToken,
    ) -> Result<Vec<u8>> {
        crate::check_cancel(token)?;
        let mut group = self.spawn(name, args)?;
        let (child, out, err) = group.parts();
        let read = async {
            let stdout_label = format!("{name}: output");
            let stderr_label = format!("{name}: stderr");
            let (stdout, stderr) = tokio::try_join!(
                read_pipe(out, limit, &stdout_label),
                read_pipe(err, 1024 * 1024, &stderr_label)
            )?;
            let status = child.wait().await?;
            if !status.success() {
                return Err(Error::Process(format!(
                    "{name}: {}",
                    cli_deps::sanitize_stderr(&String::from_utf8_lossy(&stderr))
                )));
            }
            Ok(stdout)
        };
        let result = tokio::select! {
            biased;
            _ = token.cancelled() => Err(Error::Cancelled),
            result = tokio::time::timeout(timeout, read) => result.unwrap_or_else(|_| {
                Err(Error::Process(format!(
                    "{name}: timed out after {}s",
                    timeout.as_secs()
                )))
            }),
        };
        group.terminate().await;
        result
    }
    pub async fn lines<F, Fut, E>(
        &self,
        name: &str,
        args: &[impl AsRef<OsStr>],
        timeout: Duration,
        token: &CancellationToken,
        mut on_line: F,
    ) -> std::result::Result<(), E>
    where
        F: FnMut(String) -> Fut,
        Fut: std::future::Future<Output = std::result::Result<(), E>>,
        E: From<Error> + From<std::io::Error>,
    {
        use futures::StreamExt;
        use tokio_util::codec::{FramedRead, LinesCodec};
        crate::check_cancel(token)?;
        let mut group = self.spawn(name, args)?;
        let (child, out, err) = group.parts();
        let mut stdout = FramedRead::new(out, LinesCodec::new_with_max_length(1024 * 1024));
        let mut stderr = FramedRead::new(err, LinesCodec::new_with_max_length(1024 * 1024));
        let work = async {
            let mut out_done = false;
            let mut err_done = false;
            let mut last_error = String::new();
            let mut bytes = 0;
            while !out_done || !err_done {
                let (line, is_err) = tokio::select! {line=stdout.next(),if !out_done=>(line,false),line=stderr.next(),if !err_done=>(line,true)};
                let line = line
                    .transpose()
                    .map_err(|e| Error::Process(format!("{name}: invalid progress line: {e}")))?;
                if let Some(line) = line {
                    bytes += line.len();
                    if bytes > 8 * 1024 * 1024 {
                        return Err(E::from(Error::Process(format!(
                            "{name}: excessive progress output"
                        ))));
                    }
                    if is_err {
                        last_error = cli_deps::sanitize_stderr(&line);
                    }
                    on_line(line).await?;
                } else if is_err {
                    err_done = true;
                } else {
                    out_done = true;
                }
            }
            let status = child.wait().await?;
            if status.success() {
                Ok(())
            } else {
                Err(E::from(Error::Process(format!("{name}: {last_error}"))))
            }
        };
        let result = tokio::select! {
            biased;
            _ = token.cancelled() => Err(E::from(Error::Cancelled)),
            result = tokio::time::timeout(timeout, work) => result.unwrap_or_else(|_| {
                Err(E::from(Error::Process(format!("{name}: timed out"))))
            }),
        };
        group.terminate().await;
        result
    }
    fn spawn(&self, name: &str, args: &[impl AsRef<OsStr>]) -> Result<ProcessGroup> {
        let mut command = Command::new(self.program(name)?);
        command
            .args(args)
            .current_dir(&self.cwd)
            .env("PATH", &self.path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        Ok(ProcessGroup::spawn(command, self.termination_grace)?)
    }
}
async fn read_pipe(
    pipe: impl tokio::io::AsyncRead + Unpin,
    limit: usize,
    label: &str,
) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    pipe.take(limit as u64 + 1).read_to_end(&mut bytes).await?;
    if bytes.len() > limit {
        return Err(Error::Process(format!("{label} exceeds {limit} bytes")));
    }
    Ok(bytes)
}
/// A child leading its own process group, which ends with the group.
///
/// `terminate` ends the group and waits for it. Dropping a group that was not
/// terminated starts the same termination in a Tokio blocking task, which
/// runtime shutdown waits for.
struct ProcessGroup {
    child: Option<Child>,
    /// The output pipes stay open until the group is gone: closing them early
    /// would kill a member that writes while handling SIGTERM with SIGPIPE.
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
    /// The leader's process ID, which is also the group ID. Recorded at spawn
    /// because Tokio forgets it once the leader is reaped.
    group: Option<libc::pid_t>,
    grace: Duration,
}
impl ProcessGroup {
    fn spawn(mut command: Command, grace: Duration) -> std::io::Result<Self> {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
        let mut child = command.spawn()?;
        Ok(Self {
            group: child.id().and_then(|id| libc::pid_t::try_from(id).ok()),
            stdout: child.stdout.take(),
            stderr: child.stderr.take(),
            child: Some(child),
            grace,
        })
    }
    fn parts(&mut self) -> (&mut Child, &mut ChildStdout, &mut ChildStderr) {
        (
            self.child
                .as_mut()
                .expect("the child is present until the group is terminated"),
            self.stdout.as_mut().expect("piped stdout"),
            self.stderr.as_mut().expect("piped stderr"),
        )
    }
    /// Sends SIGTERM to a group with live members and waits until every member
    /// exits, sending SIGKILL to members alive after the grace period.
    async fn terminate(&mut self) {
        if let Some(reaper) = self.start_termination() {
            // Should this wait be dropped, the blocking task still finishes, and
            // the runtime waits for it when it shuts down.
            let _ = tokio::task::spawn_blocking(move || reaper.run()).await;
        }
    }
    /// Sends SIGTERM to the group and returns the reaper that finishes ending it,
    /// or `None` when every member has already exited.
    fn start_termination(&mut self) -> Option<Reaper> {
        let mut child = self.child.take()?;
        if exited(&mut child, self.group) {
            return None;
        }
        signal_group(self.group, libc::SIGTERM);
        Some(Reaper {
            child,
            pipes: [
                self.stdout
                    .take()
                    .and_then(|pipe| pipe.into_owned_fd().ok()),
                self.stderr
                    .take()
                    .and_then(|pipe| pipe.into_owned_fd().ok()),
            ]
            .into_iter()
            .flatten()
            .filter_map(nonblocking)
            .collect(),
            group: self.group,
            grace: self.grace,
            finished: false,
        })
    }
}
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        let Some(reaper) = self.start_termination() else {
            return;
        };
        // A task the shutting-down runtime discards drops its reaper, which then
        // sends SIGKILL at once.
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn_blocking(move || reaper.run());
        } else {
            let _ = std::thread::Builder::new()
                .name("process-group-reaper".into())
                .spawn(move || reaper.run());
        }
    }
}
/// Finishes terminating a process group that has received SIGTERM.
///
/// Dropping a reaper that has not finished sends SIGKILL to the group at once.
struct Reaper {
    child: Child,
    /// Nonblocking read ends of the output pipes, drained while waiting.
    pipes: Vec<std::fs::File>,
    group: Option<libc::pid_t>,
    grace: Duration,
    finished: bool,
}
impl Reaper {
    /// Waits up to `grace` for the leader and every other group member to exit,
    /// then sends SIGKILL to the group and waits for it to empty.
    ///
    /// A process group ID is not reused while the group has a member, and polling
    /// stops as soon as the group is empty, so the signals reach only this group.
    fn run(mut self) {
        const KILL_WAIT: Duration = Duration::from_secs(1);
        if !self.wait(self.grace) {
            signal_group(self.group, libc::SIGKILL);
            // Kill the leader directly too, in case it left its group. A process
            // that still survives (for example, in uninterruptible sleep) is left
            // behind, and Tokio's orphan reaper collects the leader.
            let _ = self.child.start_kill();
            self.wait(KILL_WAIT);
        }
        self.finished = true;
    }
    /// Drains the pipes until the group is gone or `limit` passes; reports
    /// whether the group is gone.
    fn wait(&mut self, limit: Duration) -> bool {
        const POLL: Duration = Duration::from_millis(20);
        let deadline = Instant::now() + limit;
        loop {
            self.drain();
            if exited(&mut self.child, self.group) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(POLL);
        }
    }
    /// Discards up to `DRAIN_LIMIT` bytes of available output from each pipe,
    /// closing pipes that reach end of file or fail. The limit keeps a member
    /// that writes without pause from holding off the deadline checks.
    fn drain(&mut self) {
        use std::io::{ErrorKind, Read};
        const DRAIN_LIMIT: usize = 1 << 20;
        let mut buffer = [0; 8192];
        self.pipes.retain_mut(|pipe| {
            let mut drained = 0;
            while drained < DRAIN_LIMIT {
                match pipe.read(&mut buffer) {
                    Ok(0) => return false,
                    Ok(read) => drained += read,
                    Err(error) if error.kind() == ErrorKind::Interrupted => {}
                    Err(error) => return error.kind() == ErrorKind::WouldBlock,
                }
            }
            true
        });
    }
}
impl Drop for Reaper {
    fn drop(&mut self) {
        if !self.finished {
            signal_group(self.group, libc::SIGKILL);
            let _ = self.child.start_kill();
        }
    }
}
fn nonblocking(pipe: std::os::fd::OwnedFd) -> Option<std::fs::File> {
    use std::os::fd::AsRawFd;
    let fd = pipe.as_raw_fd();
    // SAFETY: fcntl(2) on a descriptor this function owns only reads and
    // updates its status flags.
    let ok = unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        flags != -1 && libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) != -1
    };
    ok.then(|| pipe.into())
}
/// Reports whether the leader has exited (reaping it) and no group member remains.
fn exited(child: &mut Child, group: Option<libc::pid_t>) -> bool {
    !matches!(child.try_wait(), Ok(None))
        // SAFETY: kill(2) with signal 0 only checks for processes in the group.
        && group.is_none_or(|group| unsafe { libc::kill(-group, 0) } != 0)
}
fn signal_group(group: Option<libc::pid_t>, signal: libc::c_int) {
    if let Some(group) = group {
        // SAFETY: kill(2) has no memory-safety preconditions; ESRCH means every
        // member already exited.
        unsafe {
            libc::kill(-group, signal);
        }
    }
}
pub fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| (*s).to_owned()).collect()
}
