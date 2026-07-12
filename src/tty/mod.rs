pub(crate) mod event;

use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;
use bytes::BytesMut;
use tokio::io::AsyncReadExt;
use tracing::Instrument;
use virtual_fs::Pipe;
use wasmer_wasix::os::Tty;
use event::AsyncEvent;



#[derive(Debug, Clone)]
pub struct TtyDevice {
    pub rx: Pipe,
    pub tx: Pipe,
    pub eof: Arc<AsyncEvent>,  /* should be private when copy_to_stdin is moved here */
}

impl TtyDevice {
    pub fn new(rx: Pipe, tx: Pipe) -> Self {
        Self {
            rx,
            tx,
            eof: Arc::new(AsyncEvent::new())
        }
    }

    pub fn attach(&self, u_stdin_rx: Pipe, tty: Tty) {
        // Because the TTY is manually copying between pipes, we need to make
        // sure the stdin pipe passed to the runtime is closed when the user
        // closes their end.
        let cleanup = {
            let mut rx = self.rx.clone();
            move || {
                tracing::debug!("Closing stdin");
                rx.close();
            }
        };

        // Use the JS event loop to drive our manual user->tty copy
        wasm_bindgen_futures::spawn_local(
            copy_stdin_to_tty(u_stdin_rx, tty, self.eof.clone(), cleanup)
                .in_current_span()
                .instrument(tracing::debug_span!("tty")),
        );
    }
}


pub(crate) fn copy_stdin_to_tty(
    mut u_stdin_rx: Pipe,
    mut tty: Tty,
    eof: Arc<AsyncEvent>,
    cleanup: impl FnOnce(),
) -> impl std::future::Future<Output = ()> {
    /// A RAII guard used to make sure the cleanup function always gets called.
    struct CleanupGuard<F: FnOnce()>(Option<F>);

    impl<F: FnOnce()> Drop for CleanupGuard<F> {
        fn drop(&mut self) {
            let cb = self.0.take().unwrap();
            cb();
        }
    }

    async move {
        let _guard = CleanupGuard(Some(cleanup));
        let mut buffer = BytesMut::new();

        loop {
            match u_stdin_rx.read_buf(&mut buffer).await {
                Ok(0) => {
                    break;
                }
                Ok(_) => {
                    tty = tty.on_event(wasmer_wasix::os::InputEvent::Raw(buffer.split().into())).await;
                    todo!()
                    //if tty.eof_take() { eof.set(); }
                }
                Err(e) => {
                    tracing::warn!(
                        error = &e as &dyn std::error::Error,
                        "Error reading stdin and copying it to the tty"
                    );
                    break;
                }
            }
        }
    }
}

impl virtual_fs::AsyncSeek for TtyDevice {
    fn start_seek(self: Pin<&mut Self>, _: std::io::SeekFrom) -> Result<(), std::io::Error> { todo!() }
    fn poll_complete(self: Pin<&mut Self>, _: &mut std::task::Context<'_>) -> Poll<Result<u64, std::io::Error>> { todo!() }
}

impl virtual_fs::AsyncRead for TtyDevice {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>,
                 buf: &mut tokio::io::ReadBuf<'_>) -> Poll<Result<(), std::io::Error>> {
        if self.eof.poll(cx) {
            Poll::Ready(Ok(()))
        }
        else {
            Pin::new(&mut self.rx).poll_read(cx, buf)
        }
    }
}

impl virtual_fs::AsyncWrite for TtyDevice {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>, buf: &[u8]) -> Poll<Result<usize, std::io::Error>> {
        Pin::new(&mut self.tx).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Result<(), std::io::Error>> {
        // `flush` is called on stdin when current process ends.
        // this, according to POSIX, should reset the eof status of the TTY.
        // note: if TtyDevice is used as both stdin and stderr, perhaps stdin does not have to
        // be flushed (which is a bit awkward as this is an `AsyncWrite` method)
        self.eof.reset();
        Pin::new(&mut self.tx).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.tx).poll_shutdown(cx)
    }
}

impl virtual_fs::VirtualFile for TtyDevice {

    fn last_accessed(&self) -> u64 { self.rx.last_accessed() }
    fn last_modified(&self) -> u64 { self.rx.last_modified() }
    fn created_time(&self) -> u64 { self.rx.created_time() }
    fn size(&self) -> u64 { self.rx.size() }
    fn set_len(&mut self, _new_size: u64) -> virtual_fs::Result<()> { Ok(()) }

    fn unlink(&mut self) -> virtual_fs::Result<()> {
        //self.eof.reset();
        Ok(())
    }

    fn poll_read_ready(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<std::io::Result<usize>> {
        tracing::trace!("poll_read_ready");
        if self.eof.is_set() { return Poll::Ready(Ok(0)); }
        Pin::new(&mut self.rx).poll_read_ready(cx)
    }
    fn poll_write_ready(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.tx).poll_write_ready(cx)
    }
}
