use once_cell::sync::OnceCell;
use tokio::{runtime::Runtime, sync::mpsc};
use tracing::{debug, error, info};

use crate::{Atman, Command, Error};

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

static COMMAND_SENDER: OnceCell<mpsc::Sender<Command>> = OnceCell::new();

async fn run() -> Result<(), Error> {
    info!("Initializing Atman...");
    let (atman, command_sender) = Atman::new()?;
    COMMAND_SENDER
        .set(command_sender)
        .map_err(|_| Error::DoubleInit("COMMAND_SENDER".into()))?;
    atman.run().await
}

/// Send a [`Command`] to Atman.
/// This accepts a JSON-represented command as a byte array,
/// converts it to a [`Command`], and sends it to Atman.
///
/// # Safety
/// `msg` must be a valid pointer to a byte array of length `len`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn send_atman_command(cmd: *const u8, len: usize) {
    if cmd.is_null() {
        error!("Received null message pointer.");
        return;
    }
    let cmd = unsafe { std::slice::from_raw_parts(cmd, len) };

    match serde_json::from_slice::<Command>(cmd) {
        Err(e) => {
            error!("Failed to parse command from JSON: {e}");
            return;
        }
        Ok(cmd) => match COMMAND_SENDER.get() {
            Some(sender) => {
                if let Err(e) = sender.blocking_send(cmd) {
                    error!("Failed to send message to Atman: {e}");
                } else {
                    debug!("Message sent to Atman: {len} bytes");
                }
            }
            None => {
                error!("Atman is not initialized. Please call run_atman first.");
            }
        },
    }
}
