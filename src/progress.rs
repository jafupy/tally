use crate::{output, result::Sink};
use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
    time::Duration,
};

pub fn show(sink: Arc<Sink>, done: Receiver<()>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut last_files = None;

        loop {
            match done.recv_timeout(Duration::from_millis(250)) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let files = sink.files();
                    if last_files == Some(files) {
                        continue;
                    }

                    last_files = Some(files);
                    eprint!(
                        "\r\x1b[36mprocessed {} files\x1b[0m",
                        output::format_number(files)
                    );
                }
            }
        }

        eprint!("\r{:<24}\r", "");
    })
}
