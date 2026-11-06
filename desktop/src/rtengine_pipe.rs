//! Keep Windows pipe writes off the request thread so cancellation never waits for stdin.
use std::{
    io::{self, Write},
    process::ChildStdin,
    sync::mpsc,
    thread::{self, JoinHandle},
};

type Payload = (Vec<u8>, Vec<u8>);
pub(super) struct WorkerInput {
    sender: Option<mpsc::SyncSender<Payload>>,
    completed: mpsc::Receiver<io::Result<()>>,
    writer: Option<JoinHandle<()>>,
}
impl WorkerInput {
    pub(super) fn new(mut stdin: ChildStdin) -> Self {
        let (sender, requests) = mpsc::sync_channel::<Payload>(1);
        let (done, completed) = mpsc::channel();
        let writer = thread::spawn(move || {
            while let Ok((header, pixels)) = requests.recv() {
                let result = stdin
                    .write_all(&header)
                    .and_then(|()| stdin.write_all(&pixels));
                let failed = result.is_err();
                if done.send(result).is_err() || failed {
                    break;
                }
            }
        });
        Self {
            sender: Some(sender),
            completed,
            writer: Some(writer),
        }
    }
    pub(super) fn send(&self, header: Vec<u8>, pixels: Vec<u8>) -> io::Result<()> {
        self.check()?;
        self.sender
            .as_ref()
            .unwrap()
            .try_send((header, pixels))
            .map_err(|error| io::Error::other(error.to_string()))
    }
    pub(super) fn check(&self) -> io::Result<()> {
        match self.completed.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => Ok(()),
            Err(mpsc::TryRecvError::Disconnected) => Err(io::ErrorKind::BrokenPipe.into()),
        }
    }
}
impl Drop for WorkerInput {
    fn drop(&mut self) {
        // Worker::drop kills and waits for the child first, releasing a blocked WriteFile.
        self.sender.take();
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

    #[test]
    fn stalled_reader_does_not_block_cancellation_and_child_exit_releases_writer() {
        let mut child = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 30",
            ])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let input = WorkerInput::new(child.stdin.take().unwrap());
        let started = Instant::now();
        input.send(vec![7; 4 * 1024 * 1024], Vec::new()).unwrap();
        thread::sleep(Duration::from_millis(30));
        assert!(started.elapsed() < Duration::from_secs(1));
        child.kill().unwrap();
        child.wait().unwrap();
        drop(input);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn large_binary_payload_survives_backpressure_without_copying_or_reordering() {
        let mut child = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command",
                "$i=[Console]::OpenStandardInput(); $o=[Console]::OpenStandardOutput(); $b=New-Object byte[] 16384; $left=4194304; while($left -gt 0) { $n=$i.Read($b,0,[Math]::Min($left,$b.Length)); if($n -eq 0){exit 1}; $o.Write($b,0,$n); $left-=$n }"])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        let input = WorkerInput::new(child.stdin.take().unwrap());
        let bytes: Vec<u8> = (0..4 * 1024 * 1024).map(|i| (i % 256) as u8).collect();
        input
            .send(bytes[..7].to_vec(), bytes[7..].to_vec())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, bytes);
        input.check().unwrap();
    }
}
