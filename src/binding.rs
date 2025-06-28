use once_cell::sync::OnceCell;
use tokio::{runtime::Runtime, sync::mpsc};

use crate::AtmanCore;

static ASYNC_RUNTIME: OnceCell<Runtime> = OnceCell::new();

fn get_async_runtime() -> &'static Runtime {
    ASYNC_RUNTIME.get_or_init(|| Runtime::new().expect("Failed to create Tokio runtime"))
}

#[unsafe(no_mangle)]
pub extern "C" fn run_atman_core() {
    get_async_runtime().spawn(async {
        if let Err(e) = run().await {
            eprintln!("Failed to run Atman Core: {e}");
        } else {
            println!("Atman Core is terminated");
        }
    });
}

static MESSAGE_SENDER: OnceCell<mpsc::Sender<Vec<u8>>> = OnceCell::new();

async fn run() -> Result<(), String> {
    println!("Initializing Atman Core...");
    let (core, message_sender) = AtmanCore::new()?;
    MESSAGE_SENDER
        .set(message_sender)
        .map_err(|_| "failed to set MESSAGE_SENDER".to_string())?;
    core.run().await;
    Ok(())
}

/// Send a message to Atman Core.
///
/// # Safety
/// `msg` must be a valid pointer to a byte array of length `len`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn send_atman_core_message(msg: *const u8, len: usize) {
    // Convert msg to Vec<u8> for easier handling
    if msg.is_null() {
        eprintln!("Received null message pointer.");
        return;
    }
    let msg = unsafe { std::slice::from_raw_parts(msg, len).to_vec() };

    match MESSAGE_SENDER.get() {
        Some(sender) => {
            if let Err(e) = sender.blocking_send(msg) {
                eprintln!("Failed to send message to Atman Core: {e}");
            } else {
                println!("Message sent to Atman Core: {len} bytes");
            }
        }
        None => {
            eprintln!("Atman Core is not initialized. Please call run_atman_core first.");
        }
    }
}
