use std::{
    ffi::{CStr, CString, c_char, c_ushort},
    path::PathBuf,
    str::FromStr as _,
    time::Duration,
};

use iroh::EndpointId;
use once_cell::sync::OnceCell;
use tokio::{
    runtime::Runtime,
    sync::{mpsc, oneshot},
};
use tracing::{debug, error, info};

use crate::{
    Atman, Command, Config,
    actors::{network, sync},
    config::secret_key_from_hex,
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
pub unsafe extern "C" fn run_atman(
    identity: *const c_char,
    network_key: *const c_char,
    custom_relay_url: *const c_char,
    syncman_dir: *const c_char,
    sync_interval_secs: u64,
) -> c_ushort {
    init_tracing_subscriber();

    if COMMAND_SENDER.get().is_some() {
        error!("Atman has been already initialized");
        return 1;
    }

    let identity = unsafe { CStr::from_ptr(identity) }
        .to_str()
        .expect("Invalid UTF-8 string for identity")
        .as_bytes();
    let Ok(identity) = secret_key_from_hex(identity) else {
        error!("Invalid identity");
        return 1;
    };

    let network_key = unsafe { CStr::from_ptr(network_key) }
        .to_str()
        .expect("Invalid UTF-8 string for network_key")
        .as_bytes();
    let Ok(network_key) = secret_key_from_hex(network_key) else {
        error!("Invalid network_key");
        return 1;
    };

    let custom_relay_url = {
        if custom_relay_url.is_null() {
            None
        } else {
            let url_str = unsafe { CStr::from_ptr(custom_relay_url) }
                .to_str()
                .expect("Invalid UTF-8 string for custom_relay_url");
            match url_str.parse() {
                Ok(url) => Some(url),
                Err(e) => {
                    error!("Invalid custom_relay_url: {e}");
                    return 1;
                }
            }
        }
    };

    let syncman_dir = unsafe { CStr::from_ptr(syncman_dir) }
        .to_str()
        .expect("Invalid UTF-8 string for syncman_dir")
        .to_string();
    let sync_interval = if sync_interval_secs == 0 {
        None
    } else {
        Some(Duration::from_secs(sync_interval_secs))
    };

    info!("Initializing Atman...");
    let (atman, command_sender) = match Atman::new(Config {
        identity,
        network: network::Config {
            key: Some(iroh::SecretKey::from_bytes(&network_key)),
            custom_relay_url,
        },
        sync: sync::Config {
            syncman_dir: PathBuf::from(syncman_dir),
        },
        sync_interval,
        #[cfg(feature = "rest")]
        rest: Default::default(),
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

/// Send a [`ConnectAndSyncCommand`] to Atman.
///
/// # Safety
/// All fields in [`ConnectAndSyncCommand`] must be valid pointers to byte
/// arrays of the corresponding length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn send_atman_connect_and_sync_command(cmd: ConnectAndSyncCommand) {
    let node_id = unsafe { std::slice::from_raw_parts(cmd.node_id, cmd.node_id_len) };
    let node_id = match EndpointId::from_str(String::from_utf8_lossy(node_id).to_string().as_str())
    {
        Ok(node_id) => node_id,
        Err(e) => {
            error!("Invalid NodeId: {e}");
            return;
        }
    };
    let doc_space = unsafe { std::slice::from_raw_parts(cmd.doc_space, cmd.doc_space_len) };
    let Ok(doc_space) = doc_space.try_into() else {
        error!("Invalid doc_space");
        return;
    };
    let doc_id = unsafe { std::slice::from_raw_parts(cmd.doc_id, cmd.doc_id_len) };
    let Ok(doc_id) = doc_id.try_into() else {
        error!("Invalid doc_id");
        return;
    };

    let (reply_sender, reply_receiver) = oneshot::channel();
    send_command(Command::ConnectAndSync {
        node_id,
        doc_space,
        doc_id,
        reply_sender,
    });

    match reply_receiver.blocking_recv() {
        Ok(Ok(())) => info!("ConnectAndSync succeeded"),
        Ok(Err(e)) => error!("ConnectAndSync failed: {e:?}"),
        Err(e) => error!("Failed to receive ConnectAndSync reply: {e:?}"),
    }
}

#[repr(C)]
pub struct ConnectAndSyncCommand {
    pub node_id: *const u8,
    pub node_id_len: usize,
    pub doc_space: *const u8,
    pub doc_space_len: usize,
    pub doc_id: *const u8,
    pub doc_id_len: usize,
}

/// Send a [`SyncUpdateCommand`] to Atman.
///
/// # Safety
/// All fields in [`SyncUpdateCommand`] must be valid pointers to byte arrays of
/// the corresponding length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn send_atman_sync_update_command(cmd: SyncUpdateCommand) {
    let doc_space = unsafe { std::slice::from_raw_parts(cmd.doc_space, cmd.doc_space_len) };
    let Ok(doc_space) = doc_space.try_into() else {
        error!("Invalid doc_space");
        return;
    };
    let doc_id = unsafe { std::slice::from_raw_parts(cmd.doc_id, cmd.doc_id_len) };
    let Ok(doc_id) = doc_id.try_into() else {
        error!("Invalid doc_id");
        return;
    };
    let data = unsafe { std::slice::from_raw_parts(cmd.data, cmd.data_len) };

    let (msg, reply_receiver) = crate::actors::sync::message::UpdateMessage {
        doc_space,
        doc_id,
        data: data.to_vec(),
    }
    .into();
    send_command(Command::Sync(msg));

    match reply_receiver.blocking_recv() {
        Ok(Ok(())) => info!("Sync update succeeded"),
        Ok(Err(e)) => error!("Sync update failed: {e:?}"),
        Err(e) => error!("Failed to receive reply: {e:?}"),
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

/// Send a [`SyncListInsertCommand`] to Atman.
///
/// # Safety
/// All fields in [`SyncListInsertCommand`] must be valid pointers to byte
/// arrays of the corresponding length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn send_atman_sync_list_insert_command(cmd: SyncListInsertCommand) {
    let doc_space = unsafe { std::slice::from_raw_parts(cmd.doc_space, cmd.doc_space_len) };
    let Ok(doc_space) = doc_space.try_into() else {
        error!("Invalid doc_space");
        return;
    };
    let collection_doc_id =
        unsafe { std::slice::from_raw_parts(cmd.collection_doc_id, cmd.collection_doc_id_len) };
    let Ok(collection_doc_id) = collection_doc_id.try_into() else {
        error!("Invalid collection_doc_id");
        return;
    };
    let doc_id = unsafe { std::slice::from_raw_parts(cmd.doc_id, cmd.doc_id_len) };
    let Ok(doc_id) = doc_id.try_into() else {
        error!("Invalid doc_id");
        return;
    };
    let property = unsafe { std::slice::from_raw_parts(cmd.property, cmd.property_len) };
    let data = unsafe { std::slice::from_raw_parts(cmd.data, cmd.data_len) };

    let (msg, reply_receiver) = crate::actors::sync::message::ListInsertMessage {
        doc_space,
        collection_doc_id,
        doc_id,
        property: String::from_utf8_lossy(property).to_string(),
        data: data.to_vec(),
        index: cmd.index,
    }
    .into();
    send_command(Command::Sync(msg));

    match reply_receiver.blocking_recv() {
        Ok(Ok(())) => info!("Sync list insert succeeded"),
        Ok(Err(e)) => error!("Sync list insert failed: {e:?}"),
        Err(e) => error!("Failed to receive reply: {e:?}"),
    }
}

#[repr(C)]
pub struct SyncListInsertCommand {
    pub doc_space: *const u8,
    pub doc_space_len: usize,
    pub collection_doc_id: *const u8,
    pub collection_doc_id_len: usize,
    pub doc_id: *const u8,
    pub doc_id_len: usize,
    pub property: *const u8,
    pub property_len: usize,
    pub data: *const u8,
    pub data_len: usize,
    pub index: usize,
}

/// Send a [`SyncGetCommand`] to Atman.
///
/// Returns a JSON represented document if successful. Otherwise, return NULL.
///
/// # Safety
/// All fields in [`SyncGetCommand`] must be valid pointers to byte arrays of
/// the corresponding length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn send_atman_sync_get_command(cmd: SyncGetCommand) -> *mut c_char {
    let doc_space = unsafe { std::slice::from_raw_parts(cmd.doc_space, cmd.doc_space_len) };
    let Ok(doc_space) = doc_space.try_into() else {
        error!("Invalid doc_space");
        return std::ptr::null_mut();
    };
    let doc_id = unsafe { std::slice::from_raw_parts(cmd.doc_id, cmd.doc_id_len) };
    let Ok(doc_id) = doc_id.try_into() else {
        error!("Invalid doc_id");
        return std::ptr::null_mut();
    };

    let (msg, reply_receiver) =
        crate::actors::sync::message::GetMessage { doc_space, doc_id }.into();
    send_command(Command::Sync(msg));

    match reply_receiver.blocking_recv() {
        Ok(Ok(document)) => {
            info!("Sync get succeeded: {document:?}");
            match document.serialize() {
                Ok(json) => {
                    let cstring =
                        CString::new(json).expect("JSON string must be converted to CString");
                    cstring.into_raw()
                }
                Err(e) => {
                    error!("JSON serialization error: {e}");
                    std::ptr::null_mut()
                }
            }
        }
        Ok(Err(e)) => {
            error!("Sync get failed: {e:?}");
            std::ptr::null_mut()
        }
        Err(e) => {
            error!("Failed to receive reply: {e:?}");
            std::ptr::null_mut()
        }
    }
}

#[repr(C)]
pub struct SyncGetCommand {
    pub doc_space: *const u8,
    pub doc_space_len: usize,
    pub doc_id: *const u8,
    pub doc_id_len: usize,
}

/// Free a C string allocated by Rust if it is not null.
///
/// # Safety
/// `str` must be a valid pointer returned by Rust.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn free_string(str: *mut c_char) {
    if !str.is_null() {
        let _cstring = unsafe { CString::from_raw(str) };
        // _cstring is dropped here.
    }
}

/// Import one or more files into atman's blob store and return a shareable
/// ticket. Returns `NULL` on error.
///
/// `file_paths` is a single C string containing newline-separated absolute
/// paths — one file per line.
///
/// # Safety
/// `file_paths` must be a valid null-terminated UTF-8 C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn send_atman_blobs_add_files_command(
    file_paths: *const c_char,
) -> *mut c_char {
    let paths = match unsafe { CStr::from_ptr(file_paths) }.to_str() {
        Ok(s) => s,
        Err(e) => {
            error!("Invalid UTF-8 file_paths: {e}");
            return std::ptr::null_mut();
        }
    };
    let paths: Vec<PathBuf> = paths
        .split('\n')
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect();
    if paths.is_empty() {
        error!("send_atman_add_blob_command: no paths supplied");
        return std::ptr::null_mut();
    }

    let (reply_sender, reply_receiver) = oneshot::channel();
    send_command(Command::SendFiles {
        paths,
        reply_sender,
    });

    match reply_receiver.blocking_recv() {
        Ok(Ok(ticket)) => {
            info!("AddBlob succeeded");
            let s = ticket.to_string();
            CString::new(s).expect("ticket string is ASCII").into_raw()
        }
        Ok(Err(e)) => {
            error!("AddBlob failed: {e:?}");
            std::ptr::null_mut()
        }
        Err(e) => {
            error!("Failed to receive AddBlob reply: {e:?}");
            std::ptr::null_mut()
        }
    }
}

/// Receiver side: parse `ticket`, pull every blob it references, and save
/// each one into `save_dir`. On success, returns the saved paths as a
/// newline-separated C string (one path per line; caller frees with
/// [`free_string`]). Returns `NULL` on error.
///
/// # Safety
/// `ticket` and `save_dir` must be valid null-terminated UTF-8 C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn send_atman_blobs_download_files_command(
    ticket: *const c_char,
    save_dir: *const c_char,
) -> *mut c_char {
    let ticket_str = match unsafe { CStr::from_ptr(ticket) }.to_str() {
        Ok(s) => s,
        Err(e) => {
            error!("Invalid UTF-8 ticket: {e}");
            return std::ptr::null_mut();
        }
    };
    let ticket = match crate::BlobTicket::from_str(ticket_str) {
        Ok(t) => t,
        Err(e) => {
            error!("Failed to parse ticket: {e}");
            return std::ptr::null_mut();
        }
    };

    let save_dir = match unsafe { CStr::from_ptr(save_dir) }.to_str() {
        Ok(s) => PathBuf::from(s),
        Err(e) => {
            error!("Invalid UTF-8 save_dir: {e}");
            return std::ptr::null_mut();
        }
    };

    let (reply_sender, reply_receiver) = oneshot::channel();
    send_command(Command::DownloadFiles {
        ticket,
        save_dir,
        reply_sender,
    });

    match reply_receiver.blocking_recv() {
        Ok(Ok(paths)) => {
            info!(count = paths.len(), "DownloadBlob succeeded");
            let joined = paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("\n");
            CString::new(joined)
                .expect("paths contain no null bytes")
                .into_raw()
        }
        Ok(Err(e)) => {
            error!("DownloadBlob failed: {e:?}");
            std::ptr::null_mut()
        }
        Err(e) => {
            error!("Failed to receive DownloadBlob reply: {e:?}");
            std::ptr::null_mut()
        }
    }
}

/// Returns how many distinct receivers have fully pulled the ticket.
/// Returns 0 if the ticket is unknown.
///
/// # Safety
/// `ticket` must be a valid null-terminated UTF-8 C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn send_atman_blobs_files_transfer_count_command(
    ticket: *const c_char,
) -> u64 {
    let ticket = match unsafe { CStr::from_ptr(ticket) }.to_str() {
        Ok(s) => s,
        Err(e) => {
            error!("Invalid UTF-8 ticket: {e}");
            return 0;
        }
    };
    let ticket = match crate::BlobTicket::from_str(ticket) {
        Ok(t) => t,
        Err(e) => {
            error!("Failed to parse ticket: {e}");
            return 0;
        }
    };

    let (reply_sender, reply_receiver) = oneshot::channel();
    send_command(Command::FilesTransferCount {
        hash: ticket.inner.hash(),
        reply_sender,
    });
    match reply_receiver.blocking_recv() {
        Ok(count) => count.unwrap_or(0),
        Err(e) => {
            error!("Failed to receive TransferCount reply: {e:?}");
            0
        }
    }
}
