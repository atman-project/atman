use std::{
    ffi::{CStr, c_char, c_ushort},
    path::PathBuf,
};

use once_cell::sync::OnceCell;
use tokio::{
    runtime::Runtime,
    sync::{mpsc, oneshot},
};
use tracing::{debug, error, info};

use crate::{
    Atman, Command, Config, Error,
    actors::{network, sync},
};

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

/// Initialize and run Atman with the given syncman directory.
///
/// # Safety
/// `syncman_dir` must be a valid null-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn run_atman(syncman_dir: *const c_char) -> c_ushort {
    init_tracing_subscriber();

    if COMMAND_SENDER.get().is_some() {
        error!("{:?}", Error::DoubleInit("Atman".into()));
        return 1;
    }

    let syncman_dir = unsafe { CStr::from_ptr(syncman_dir) }
        .to_str()
        .expect("Invalid UTF-8 string for syncman_dir")
        .to_string();

    info!("Initializing Atman...");
    let (atman, command_sender) = match Atman::new(Config {
        network: network::Config { key: None },
        sync: sync::Config {
            syncman_dir: PathBuf::from(syncman_dir),
            overwrite: false,
        },
    }) {
        Ok(result) => result,
        Err(e) => {
            error!("Failed to initialize Atman: {e}");
            return 1;
        }
    };
    COMMAND_SENDER
        .set(command_sender)
        .expect("COMMAND_SENDER should be empty");

    let (ready_sender, ready_receiver) = oneshot::channel();
    get_async_runtime().spawn(async { atman.run(ready_sender).await });
    match ready_receiver
        .blocking_recv()
        .expect("ready channel shouldn't be closed")
    {
        Ok(()) => {
            info!("Atman is ready.");
            0
        }
        Err(e) => {
            error!("Failed to run Atman: {e}");
            1
        }
    }
}

static COMMAND_SENDER: OnceCell<mpsc::Sender<Command>> = OnceCell::new();

fn send_command(cmd: Command) {
    match COMMAND_SENDER.get() {
        Some(sender) => {
            if let Err(e) = sender.blocking_send(cmd) {
                error!("Failed to send command to Atman: {e}");
            } else {
                debug!("Command sent to Atman");
            }
        }
        None => {
            error!("Atman is not initialized. Please call run_atman first.");
        }
    }
}

/// Send a [`SyncUpdateCommand`] to Atman.
///
/// # Safety
/// all fields in [`SyncUpdateCommand`] must be valid pointers to byte arrays of
/// the corresponding length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn send_atman_sync_update_command(cmd: SyncUpdateCommand) {
    let doc_space = unsafe { std::slice::from_raw_parts(cmd.doc_space, cmd.doc_space_len) };
    let doc_id = unsafe { std::slice::from_raw_parts(cmd.doc_id, cmd.doc_id_len) };
    let data = unsafe { std::slice::from_raw_parts(cmd.data, cmd.data_len) };
    send_command(Command::Sync(
        crate::actors::sync::UpdateMessage {
            doc_space: String::from_utf8_lossy(doc_space).to_string().into(),
            doc_id: String::from_utf8_lossy(doc_id).to_string().into(),
            data: data.to_vec(),
        }
        .into(),
    ));
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

/// Send a [`SyncListInsertCommand`] to Atman.
///
/// # Safety
/// all fields in [`SyncListInsertCommand`] must be valid pointers to byte
/// arrays of the corresponding length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn send_atman_sync_list_insert_command(cmd: SyncListInsertCommand) {
    let doc_space = unsafe { std::slice::from_raw_parts(cmd.doc_space, cmd.doc_space_len) };
    let doc_id = unsafe { std::slice::from_raw_parts(cmd.doc_id, cmd.doc_id_len) };
    let property = unsafe { std::slice::from_raw_parts(cmd.property, cmd.property_len) };
    let data = unsafe { std::slice::from_raw_parts(cmd.data, cmd.data_len) };
    send_command(Command::Sync(
        crate::actors::sync::ListInsertMessage {
            doc_space: String::from_utf8_lossy(doc_space).to_string().into(),
            doc_id: String::from_utf8_lossy(doc_id).to_string().into(),
            property: String::from_utf8_lossy(property).to_string(),
            data: data.to_vec(),
            index: cmd.index,
        }
        .into(),
    ));
}

#[repr(C)]
pub struct SyncListInsertCommand {
    pub doc_space: *const u8,
    pub doc_space_len: usize,
    pub doc_id: *const u8,
    pub doc_id_len: usize,
    pub property: *const u8,
    pub property_len: usize,
    pub data: *const u8,
    pub data_len: usize,
    pub index: usize,
}
