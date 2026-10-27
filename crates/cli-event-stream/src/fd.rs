use std::{
    io::{self, Write},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    pin::Pin,
    task::{Context, Poll, ready},
};
use tokio::io::{AsyncWrite, unix::AsyncFd};

pub(super) fn writer(raw: i32) -> io::Result<Box<dyn AsyncWrite + Send + Unpin>> {
    // Duplicate ownership, preserving the caller's descriptor even on failure.
    let duplicate = unsafe { libc::fcntl(raw, libc::F_DUPFD_CLOEXEC, 0) };
    if duplicate < 0 {
        return Err(io::Error::last_os_error());
    }
    let owned = unsafe { OwnedFd::from_raw_fd(duplicate) };
    let file = std::fs::File::from(owned);
    if file.metadata()?.is_file() {
        return Ok(Box::new(tokio::fs::File::from_std(file)));
    }
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let restore = Nonblocking { file, flags };
    // SAFETY: Nonblocking owns this stable descriptor and exposes no operation
    // that can replace or close it before AsyncFd deregisters it.
    Ok(Box::new(Writer(
        unsafe { AsyncFd::register(restore) }.map_err(io::Error::from)?,
    )))
}

struct Nonblocking {
    file: std::fs::File,
    flags: i32,
}
impl AsRawFd for Nonblocking {
    fn as_raw_fd(&self) -> i32 {
        self.file.as_raw_fd()
    }
}
impl Drop for Nonblocking {
    fn drop(&mut self) {
        // O_NONBLOCK belongs to the shared open-file description. Restore it
        // before releasing our duplicate, also on registration/write failure.
        unsafe { libc::fcntl(self.as_raw_fd(), libc::F_SETFL, self.flags) };
    }
}
struct Writer(AsyncFd<Nonblocking>);
impl AsyncWrite for Writer {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            let mut readiness = ready!(self.0.poll_write_ready(cx))?;
            match readiness.try_io(|inner| (&inner.get_ref().file).write(bytes)) {
                Ok(Err(error)) if error.kind() == io::ErrorKind::Interrupted => continue,
                Ok(result) => return Poll::Ready(result),
                Err(_) => continue,
            }
        }
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        // Closing this duplicate must not shut down the caller's socket.
        Poll::Ready(Ok(()))
    }
}
