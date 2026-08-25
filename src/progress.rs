use crate::{output::format_number, sink::Sink};
use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread::JoinHandle,
    time::Duration,
};

pub type Progress = (Sender<()>, JoinHandle<()>);

pub fn start(sink: Arc<Sink>) -> Progress {
    let (done, receiver) = mpsc::channel();
    (done, show(sink, receiver))
}

pub fn stop((done, progress): Progress) {
    let _ = done.send(());
    progress.join().unwrap();
}

fn show(sink: Arc<Sink>, done: Receiver<()>) -> JoinHandle<()> {
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
                    eprint!("\r\x1b[36mprocessed {} files\x1b[0m", format_number(files));
                }
            }
        }

        eprint!("\r{:<24}\r", "");
    })
}
