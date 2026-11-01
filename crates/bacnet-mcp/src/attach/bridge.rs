//! `bacnet mcp connect`'s side: an agent host's byte stream passed to the
//! TUI's socket (or named pipe) and back, byte for byte, with no MCP
//! parsing of its own. The connection, and its check that the listener is
//! this user's own, is the platform's ([`connect`]).

use std::io;
use std::path::Path;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _};

use super::{AttachError, connect};

/// Pass `input` to the TUI's socket at `path` and what comes back to
/// `output`, until the TUI ends the session. The end of `input` is passed
/// on, and the bridge then waits for the TUI to finish answering.
pub async fn bridge<R, W>(path: &Path, input: R, output: W) -> Result<(), AttachError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let stream = connect(path).await?;
    pump(stream, input, output)
        .await
        .map_err(|e| AttachError(format!("the connection to the TUI failed: {e}")))
}

/// [`bridge`] on this process's stdin and stdout. A read of stdin may
/// still be blocked on a thread when it returns, so the caller should
/// exit the process then rather than wait for the runtime to wind down.
#[cfg(feature = "stdio")]
pub async fn bridge_stdio(path: &Path) -> Result<(), AttachError> {
    bridge(path, tokio::io::stdin(), tokio::io::stdout()).await
}

/// Copy both ways until `stream`'s side ends.
async fn pump<S, R, W>(stream: S, mut input: R, mut output: W) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite,
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let (mut from_tui, mut to_tui) = tokio::io::split(stream);
    let up = async {
        let copied = tokio::io::copy(&mut input, &mut to_tui).await;
        // The host closed its side: say so, as closing stdin would.
        let closed = to_tui.shutdown().await;
        copied.and(closed)
    };
    let down = async {
        tokio::io::copy(&mut from_tui, &mut output).await?;
        output.flush().await
    };
    tokio::pin!(up, down);
    // A named pipe has no half-close: the TUI sees the host's end only
    // when the pipe closes, so there the bridge ends with the host's side.
    let half_close = cfg!(unix);
    let mut up_done = false;
    loop {
        // Both copies are polled together; neither holds a lock.
        tokio::select! {
            done = &mut down => return done,
            done = &mut up, if !up_done => {
                up_done = true;
                if let Err(error) = done {
                    tracing::debug!("the host's side of the bridge ended: {error}");
                }
                if !half_close {
                    return Ok(());
                }
            }
        }
    }
}
