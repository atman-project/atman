use tokio::sync::mpsc;

pub mod binding;

struct AtmanCore {
    message_receiver: mpsc::Receiver<Vec<u8>>,
}

impl AtmanCore {
    fn new() -> Result<(Self, mpsc::Sender<Vec<u8>>), String> {
        let (message_sender, message_receiver) = mpsc::channel(100);
        Ok((Self { message_receiver }, message_sender))
    }

    async fn run(mut self) {
        println!("Atman Core is running...");
        loop {
            if let Some(message) = self.message_receiver.recv().await {
                println!("Message received: {} bytes: {:?}", message.len(), message);
            }
        }
    }
}
