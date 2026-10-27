use crate::{CancellationToken, Error, Result};
use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command};

#[derive(Clone, Debug)]
pub struct Tools {
    pub path: OsString,
    pub cwd: PathBuf,
}
impl Tools {
    pub fn new(path: OsString, cwd: PathBuf) -> Self {
        Self { path, cwd }
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
        let mut command = Command::new(self.program(name)?);
        command
            .args(args)
            .current_dir(&self.cwd)
            .env("PATH", &self.path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
        let mut child = command.spawn()?;
        let _group = ProcessGroup(child.id().expect("running child") as i32);
        let out = child.stdout.take().expect("piped stdout");
        let err = child.stderr.take().expect("piped stderr");
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
        tokio::select! {biased;_ = token.cancelled()=>Err(Error::Cancelled),result=tokio::time::timeout(timeout,read)=>result.map_err(|_|Error::Process(format!("{name}: timed out after {}s",timeout.as_secs())))?}
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
        let mut command = Command::new(self.program(name)?);
        command
            .args(args)
            .current_dir(&self.cwd)
            .env("PATH", &self.path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
        let mut child = command.spawn()?;
        let _group = ProcessGroup(child.id().expect("running child") as i32);
        let mut stdout = FramedRead::new(
            child.stdout.take().expect("piped stdout"),
            LinesCodec::new_with_max_length(1024 * 1024),
        );
        let mut stderr = FramedRead::new(
            child.stderr.take().expect("piped stderr"),
            LinesCodec::new_with_max_length(1024 * 1024),
        );
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
        tokio::select! {biased;_=token.cancelled()=>Err(E::from(Error::Cancelled)),r=tokio::time::timeout(timeout,work)=>r.map_err(|_|E::from(Error::Process(format!("{name}: timed out"))))?}
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
struct ProcessGroup(i32);
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-self.0, libc::SIGKILL);
        }
    }
}
pub fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| (*s).to_owned()).collect()
}
