//! Bounded concurrent stdout and stderr collection.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::task::JoinHandle;

use crate::MAX_OUTPUT_BYTES;

const POST_EXIT_DRAIN: Duration = Duration::from_secs(1);

pub(crate) type Capture = Arc<Mutex<CapturedOutput>>;

pub(crate) fn new_capture() -> Capture {
    Arc::new(Mutex::new(CapturedOutput::default()))
}

pub(crate) fn start_capture<R>(stream: R, capture: Capture) -> JoinHandle<()>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(capture_output(stream, capture))
}

pub(crate) async fn finish_capture(
    stdout_task: Option<JoinHandle<()>>,
    stderr_task: Option<JoinHandle<()>>,
) {
    let deadline = tokio::time::Instant::now() + POST_EXIT_DRAIN;
    for mut task in [stdout_task, stderr_task].into_iter().flatten() {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if tokio::time::timeout(remaining, &mut task).await.is_err() {
            task.abort();
            let _ = task.await;
        }
    }
}

pub(crate) async fn abort_capture(
    stdout_task: Option<JoinHandle<()>>,
    stderr_task: Option<JoinHandle<()>>,
) {
    for task in [stdout_task, stderr_task].into_iter().flatten() {
        task.abort();
        let _ = task.await;
    }
}

pub(crate) fn captured_output(output: &Capture) -> String {
    let Ok(output) = output.lock() else {
        return "Output capture was unavailable.".into();
    };
    let mut text = String::from_utf8_lossy(&output.bytes).into_owned();
    if output.truncated {
        text.push_str("\n[output truncated at 65536 bytes]");
    }
    text
}

#[derive(Default)]
pub(crate) struct CapturedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

async fn capture_output<R>(mut stream: R, output: Capture)
where
    R: AsyncRead + Unpin,
{
    let mut buffer = [0_u8; 4096];
    loop {
        let read = match stream.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let Ok(mut output) = output.lock() else {
            break;
        };
        let room = MAX_OUTPUT_BYTES.saturating_sub(output.bytes.len());
        let captured = room.min(read);
        output.bytes.extend_from_slice(&buffer[..captured]);
        output.truncated |= captured < read;
    }
}
