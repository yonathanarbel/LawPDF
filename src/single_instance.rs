use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, unbounded};

const LAWPDF_INSTANCE_ADDR: &str = "127.0.0.1:47471";
const MAGIC: &[u8] = b"LAWPDF_OPEN_V1\0";
const ACK: &[u8; 3] = b"OK\n";
const MAX_PATHS_PER_MESSAGE: usize = 256;
const MAX_PATH_BYTES: usize = 32 * 1024;
const IPC_TIMEOUT: Duration = Duration::from_secs(2);
static REPAINT_CONTEXT: OnceLock<egui::Context> = OnceLock::new();

pub enum InstanceMode {
    Primary {
        incoming_paths_tx: Sender<Vec<PathBuf>>,
        incoming_paths_rx: Receiver<Vec<PathBuf>>,
    },
    SecondarySent,
}

pub fn initialize(startup_paths: &[PathBuf]) -> InstanceMode {
    match TcpListener::bind(LAWPDF_INSTANCE_ADDR) {
        Ok(listener) => {
            let (incoming_paths_tx, incoming_paths_rx) = unbounded();
            spawn_listener(listener, incoming_paths_tx.clone());
            InstanceMode::Primary {
                incoming_paths_tx,
                incoming_paths_rx,
            }
        }
        Err(_) if send_paths_to_primary(startup_paths) => InstanceMode::SecondarySent,
        Err(_) => {
            let (incoming_paths_tx, incoming_paths_rx) = unbounded();
            InstanceMode::Primary {
                incoming_paths_tx,
                incoming_paths_rx,
            }
        }
    }
}

fn spawn_listener(listener: TcpListener, incoming_paths_tx: Sender<Vec<PathBuf>>) {
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let _ = stream.set_read_timeout(Some(IPC_TIMEOUT));
            let _ = stream.set_write_timeout(Some(IPC_TIMEOUT));

            let Ok(paths) = read_message(&mut stream) else {
                continue;
            };
            if incoming_paths_tx.send(paths).is_err() {
                break;
            }
            request_repaint();
            let _ = stream.write_all(ACK);
        }
    });
}

pub fn set_repaint_context(ctx: &egui::Context) {
    let _ = REPAINT_CONTEXT.set(ctx.clone());
}

pub fn request_repaint() {
    if let Some(ctx) = REPAINT_CONTEXT.get() {
        ctx.request_repaint();
    }
}

fn send_paths_to_primary(paths: &[PathBuf]) -> bool {
    let Ok(mut stream) = TcpStream::connect(LAWPDF_INSTANCE_ADDR) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(IPC_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IPC_TIMEOUT));

    if write_message(&mut stream, paths).is_err() {
        return false;
    }

    let mut ack = [0_u8; ACK.len()];
    stream.read_exact(&mut ack).is_ok() && &ack == ACK
}

fn write_message(stream: &mut impl Write, paths: &[PathBuf]) -> io::Result<()> {
    let payloads = paths
        .iter()
        .take(MAX_PATHS_PER_MESSAGE)
        .filter_map(|path| {
            let bytes = path.as_os_str().to_string_lossy().into_owned().into_bytes();
            (bytes.len() <= MAX_PATH_BYTES).then_some(bytes)
        })
        .collect::<Vec<_>>();

    stream.write_all(MAGIC)?;
    stream.write_all(&(payloads.len() as u32).to_le_bytes())?;
    for payload in payloads {
        stream.write_all(&(payload.len() as u32).to_le_bytes())?;
        stream.write_all(&payload)?;
    }
    stream.flush()
}

fn read_message(stream: &mut impl Read) -> io::Result<Vec<PathBuf>> {
    let mut magic = vec![0_u8; MAGIC.len()];
    stream.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected LawPDF IPC magic",
        ));
    }

    let count = read_u32(stream)? as usize;
    if count > MAX_PATHS_PER_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "too many paths in LawPDF IPC message",
        ));
    }

    let mut paths = Vec::with_capacity(count);
    for _ in 0..count {
        let len = read_u32(stream)? as usize;
        if len > MAX_PATH_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "path is too large in LawPDF IPC message",
            ));
        }
        let mut bytes = vec![0_u8; len];
        stream.read_exact(&mut bytes)?;
        let path = String::from_utf8(bytes)
            .map(PathBuf::from)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        paths.push(path);
    }
    Ok(paths)
}

fn read_u32(stream: &mut impl Read) -> io::Result<u32> {
    let mut bytes = [0_u8; 4];
    stream.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn message_frame_round_trips_paths() {
        let expected = vec![
            PathBuf::from(r"C:\cases\alpha.pdf"),
            PathBuf::from("relative/unicode-β.pdf"),
        ];
        let mut frame = Vec::new();
        write_message(&mut frame, &expected).unwrap();

        let actual = read_message(&mut Cursor::new(frame)).unwrap();

        assert_eq!(actual, expected);
    }

    #[test]
    fn message_frame_rejects_oversized_path_length() {
        let mut frame = MAGIC.to_vec();
        frame.extend_from_slice(&1_u32.to_le_bytes());
        frame.extend_from_slice(&((MAX_PATH_BYTES + 1) as u32).to_le_bytes());

        let error = read_message(&mut Cursor::new(frame)).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("path is too large"));
    }

    #[test]
    fn message_frame_rejects_bad_magic_and_malformed_payload() {
        let mut bad_magic = vec![0_u8; MAGIC.len()];
        bad_magic.extend_from_slice(&0_u32.to_le_bytes());
        assert_eq!(
            read_message(&mut Cursor::new(bad_magic))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );

        let mut invalid_utf8 = MAGIC.to_vec();
        invalid_utf8.extend_from_slice(&1_u32.to_le_bytes());
        invalid_utf8.extend_from_slice(&1_u32.to_le_bytes());
        invalid_utf8.push(0xff);
        assert_eq!(
            read_message(&mut Cursor::new(invalid_utf8))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }
}
