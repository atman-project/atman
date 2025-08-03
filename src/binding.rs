use once_cell::sync::OnceCell;
use tokio::{runtime::Runtime, sync::mpsc};
use tracing::{debug, error, info, warn};

use crate::{Atman, Command, Config, Error, SyncCommand};

static ASYNC_RUNTIME: OnceCell<Runtime> = OnceCell::new();
static TRACING_SUBSCRIBER: OnceCell<()> = OnceCell::new();

fn get_async_runtime() -> &'static Runtime {
    ASYNC_RUNTIME.get_or_init(|| Runtime::new().expect("Failed to create Tokio runtime"))
}

fn init_tracing_subscriber() {
    TRACING_SUBSCRIBER.get_or_init(|| {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
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
    let (atman, command_sender) = Atman::new(Config { iroh_key: None });
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

/// Send a [`SyncUpdateCommand`] to Atman.
///
/// # Safety
/// all fields in [`SyncUpdateCommand`] must be valid pointers to byte arrays of the corresponding length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn send_atman_sync_update_command(cmd: SyncUpdateCommand) {
    let doc_space = unsafe { std::slice::from_raw_parts(cmd.doc_space, cmd.doc_space_len) };
    let doc_id = unsafe { std::slice::from_raw_parts(cmd.doc_id, cmd.doc_id_len) };
    let data = unsafe { std::slice::from_raw_parts(cmd.data, cmd.data_len) };
    let cmd = crate::SyncUpdateCommand {
        doc_space: String::from_utf8_lossy(doc_space).to_string().into(),
        doc_id: String::from_utf8_lossy(doc_id).to_string().into(),
        data: data.to_vec(),
    };
    warn!("{cmd:?}");
    warn!("DATA: {}", String::from_utf8_lossy(&cmd.data).to_string());
    match COMMAND_SENDER.get() {
        Some(sender) => {
            if let Err(e) = sender.blocking_send(Command::Sync(SyncCommand::Update(cmd))) {
                error!("Failed to send sync update command to Atman: {e}");
            } else {
                debug!("Sync update command sent to Atman");
            }
        }
        None => {
            error!("Atman is not initialized. Please call run_atman first.");
        }
    }
}

#[repr(C)]
pub struct SyncUpdateCommand {
    pub doc_space: *const u8,
    pub doc_space_len: usize,
    pub doc_id: *const u8,
    pub doc_id_len: usize,
    pub data: *const u8,
    pub data_len: usize,
}
