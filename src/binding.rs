use once_cell::sync::OnceCell;
use tokio::{runtime::Runtime, sync::mpsc};
use tracing::{debug, error, info};

use crate::Atman;

static ASYNC_RUNTIME: OnceCell<Runtime> = OnceCell::new();
static TRACING_SUBSCRIBER: OnceCell<()> = OnceCell::new();

fn get_async_runtime() -> &'static Runtime {
    ASYNC_RUNTIME.get_or_init(|| Runtime::new().expect("Failed to create Tokio runtime"))
}

fn init_tracing_subscriber() {
    TRACING_SUBSCRIBER.get_or_init(|| {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            // Disable ANSI colors since most envs where C bindings are used don't support them.
            .with_ansi(false)
            .init();
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn run_atman() {
    init_tracing_subscriber();
    get_async_runtime().spawn(async {
        if let Err(e) = run().await {
            error!("Failed to run Atman: {e}");
        } else {
            info!("Atman is terminated");
        }
    });
}

static MESSAGE_SENDER: OnceCell<mpsc::Sender<Vec<u8>>> = OnceCell::new();

async fn run() -> Result<(), String> {
    info!("Initializing Atman...");
    let (atman, message_sender) = Atman::new()?;
    MESSAGE_SENDER
        .set(message_sender)
        .map_err(|_| "failed to set MESSAGE_SENDER".to_string())?;
    atman.run().await;
    Ok(())
}

/// Send a message to Atman.
///
/// # Safety
/// `msg` must be a valid pointer to a byte array of length `len`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn send_atman_message(msg: *const u8, len: usize) {
    // Convert msg to Vec<u8> for easier handling
    if msg.is_null() {
        error!("Received null message pointer.");
        return;
    }
    let msg = unsafe { std::slice::from_raw_parts(msg, len).to_vec() };

    match MESSAGE_SENDER.get() {
        Some(sender) => {
            if let Err(e) = sender.blocking_send(msg) {
                error!("Failed to send message to Atman: {e}");
            } else {
                debug!("Message sent to Atman: {len} bytes");
            }
        }
        None => {
            error!("Atman is not initialized. Please call run_atman first.");
        }
    }
}
