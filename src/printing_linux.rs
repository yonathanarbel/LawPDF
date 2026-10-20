//! Linux printing adapted from Sarah Lawsky's LawPDF Linux contribution.
//! CUPS discovery, PDF export, and spooling happen only after a print request
//! and off the UI thread. The private annotated copy lives until lp exits.
use std::cell::RefCell;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, bounded};

use crate::document_store::{DocumentStore, FileRevision};
use crate::model::EditorAnnotation;

pub(crate) struct PrintCopy {
    _directory: tempfile::TempDir,
    path: PathBuf,
    title: String,
    printers: Vec<Printer>,
}

impl PrintCopy {
    pub(crate) fn title(&self) -> &str {
        &self.title
    }
    pub(crate) fn printers(&self) -> &[Printer] {
        &self.printers
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Printer {
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PrintOptions {
    pub printer: Option<String>,
    pub copies: u32,
    /// Comma-separated page numbers and inclusive ranges, e.g. 1-4,7.
    pub page_range: String,
}

impl Default for PrintOptions {
    fn default() -> Self {
        Self {
            printer: None,
            copies: 1,
            page_range: String::new(),
        }
    }
}

pub(crate) fn prepare_copy(
    store: &DocumentStore,
    source: &Path,
    revision: &FileRevision,
    annotations: &[EditorAnnotation],
) -> Result<PrintCopy, String> {
    let original = store.print_source(revision)?;
    match crate::pdf_backend::allows_printing_at(&original) {
        Ok(true) => {}
        Ok(false) => return Err("This PDF does not allow printing.".to_owned()),
        Err(error) => return Err(format!("Could not read this PDF for printing: {error:#}")),
    }
    let printers = list_printers()?;
    if printers.is_empty() {
        return Err("No printer is configured. Add one in your system's printer settings, then print again.".to_owned());
    }
    let directory = tempfile::Builder::new()
        .prefix("lawpdf-print-")
        .tempdir()
        .map_err(|error| format!("Could not prepare the print copy: {error}"))?;
    let path = directory.path().join("document.pdf");
    store.export_annotations(source, revision, annotations, &path)?;
    let title = source
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    Ok(PrintCopy {
        _directory: directory,
        path,
        title,
        printers,
    })
}

/// Called by the preparation worker, never by an egui draw/update callback.
fn list_printers() -> Result<Vec<Printer>, String> {
    let output = run_cups("lpstat", &["-e"], Duration::from_secs(10))?;
    if !output.status.success() {
        return Err(service_error("Could not list printers", &output));
    }
    let default = run_cups("lpstat", &["-d"], Duration::from_secs(10))
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            output
                .stdout
                .split_once(':')
                .map(|(_, name)| name.trim().to_owned())
        });
    let mut printers: Vec<_> = output
        .stdout
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| Printer {
            name: name.to_owned(),
            is_default: default.as_deref() == Some(name),
        })
        .collect();
    printers.sort_by(|left, right| {
        right
            .is_default
            .cmp(&left.is_default)
            .then_with(|| left.name.cmp(&right.name))
    });
    printers.dedup_by(|left, right| left.name == right.name);
    Ok(printers)
}

pub(crate) fn page_range_is_valid(range: &str) -> bool {
    let range = range.trim();
    range.is_empty()
        || range.split(',').all(|part| {
            let part = part.trim();
            match part.split_once('-') {
                Some((start, end)) => matches!(
                    (start.trim().parse::<u32>(), end.trim().parse::<u32>()),
                    (Ok(start), Ok(end)) if start >= 1 && start <= end
                ),
                None => part.parse::<u32>().is_ok_and(|page| page >= 1),
            }
        })
}

/// A worker owns the PDF until CUPS finishes accepting or refusing it. Dropping
/// this handle never waits on a printer service or deletes a file lp is reading.
pub(crate) struct PrintJob {
    rx: Receiver<Result<String, String>>,
    reported: RefCell<bool>,
}

impl PrintJob {
    pub(crate) fn start_with(copy: PrintCopy, options: &PrintOptions) -> Result<Self, String> {
        if !page_range_is_valid(&options.page_range) {
            return Err("Enter pages as numbers and ranges, for example 1-4,7.".to_owned());
        }
        if !(1..=99).contains(&options.copies) {
            return Err("Choose between 1 and 99 copies.".to_owned());
        }
        let options = options.clone();
        let (tx, rx) = bounded(1);
        std::thread::Builder::new()
            .name("lawpdf-print".to_owned())
            .spawn(move || {
                let result = spool(&copy, &options);
                let _ = tx.send(result);
            })
            .map_err(|error| format!("Could not start printing: {error}"))?;
        Ok(Self {
            rx,
            reported: RefCell::new(false),
        })
    }

    /// Success means queued, never proof that the printer produced paper.
    pub(crate) fn take_result(&self) -> Option<Result<String, String>> {
        if *self.reported.borrow() {
            return None;
        }
        match self.rx.try_recv() {
            Ok(result) => {
                *self.reported.borrow_mut() = true;
                Some(result)
            }
            Err(crossbeam_channel::TryRecvError::Empty) => None,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                *self.reported.borrow_mut() = true;
                Some(Err("The printing worker stopped unexpectedly. Check the print queue before retrying.".to_owned()))
            }
        }
    }
}

fn spool(copy: &PrintCopy, options: &PrintOptions) -> Result<String, String> {
    let mut args = Vec::new();
    if let Some(printer) = options.printer.as_deref().filter(|name| !name.is_empty()) {
        args.extend(["-d".to_owned(), printer.to_owned()]);
    }
    args.extend([
        "-t".to_owned(),
        copy.title.clone(),
        "-n".to_owned(),
        options.copies.to_string(),
    ]);
    if !options.page_range.trim().is_empty() {
        args.extend([
            "-o".to_owned(),
            format!(
                "page-ranges={}",
                options.page_range.split_whitespace().collect::<String>()
            ),
        ]);
    }
    args.push("--".to_owned());
    // The temp path is absolute and controlled by tempfile, with no shell.
    args.push(
        copy.path
            .to_str()
            .ok_or_else(|| "The temporary print path is not valid UTF-8.".to_owned())?
            .to_owned(),
    );
    let args: Vec<_> = args.iter().map(String::as_str).collect();
    let output = run_cups("lp", &args, Duration::from_secs(60))?;
    if !output.status.success() {
        return Err(service_error(
            "The printing service refused the job",
            &output,
        ));
    }
    Ok(match request_id(&output.stdout) {
        Some(id) => format!("Queued for printing ({id})."),
        None => "Queued for printing.".to_owned(),
    })
}

fn request_id(output: &str) -> Option<&str> {
    output
        .strip_prefix("request id is ")?
        .split_whitespace()
        .next()
}

#[derive(Debug)]
struct ServiceOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

/// Capture service errors without a pipe that could fill while waiting. This
/// helper runs exclusively on a worker. A timeout keeps a broken CUPS server
/// from holding the app's shutdown guard forever.
fn run_cups(program: &str, args: &[&str], timeout: Duration) -> Result<ServiceOutput, String> {
    let stdout =
        tempfile::tempfile().map_err(|error| format!("Could not prepare printing: {error}"))?;
    let stderr =
        tempfile::tempfile().map_err(|error| format!("Could not prepare printing: {error}"))?;
    let mut stdout_reader = stdout.try_clone().map_err(|error| error.to_string())?;
    let mut stderr_reader = stderr.try_clone().map_err(|error| error.to_string())?;
    let mut child = Command::new(program).args(args).env("LC_ALL", "C")
        .stdin(Stdio::null()).stdout(stdout).stderr(stderr).spawn()
        .map_err(|error| format!("Could not start the printing service. Install the CUPS client commands (lp and lpstat): {error}"))?;
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(
                    "The printing service timed out. Check the print queue before retrying."
                        .to_owned(),
                );
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "Could not read the printing service result: {error}. Check the print queue before retrying."
                ));
            }
        }
    };
    use std::io::{Seek, SeekFrom};
    let _ = stdout_reader.seek(SeekFrom::Start(0));
    let _ = stderr_reader.seek(SeekFrom::Start(0));
    let mut stdout = String::new();
    let mut stderr = String::new();
    let _ = stdout_reader.take(8192).read_to_string(&mut stdout);
    let _ = stderr_reader.take(8192).read_to_string(&mut stderr);
    Ok(ServiceOutput {
        status,
        stdout,
        stderr,
    })
}

fn service_error(context: &str, output: &ServiceOutput) -> String {
    let detail = output.stderr.trim();
    if detail.is_empty() {
        format!("{context} ({}).", output.status)
    } else {
        format!("{context}: {detail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn page_ranges_accept_pages_and_spans() {
        for range in ["", "1", "1-4", "1-4,7", " 2 , 5-6 "] {
            assert!(page_range_is_valid(range), "{range}");
        }
    }
    #[test]
    fn page_ranges_reject_malformed_input() {
        for range in ["0", "4-1", "1-", "a", "1;2", "1 && reboot", "1,,2", "1-2-3"] {
            assert!(!page_range_is_valid(range), "{range}");
        }
    }
    #[test]
    fn cups_receipt_is_a_queue_identifier() {
        assert_eq!(
            request_id("request id is office-123 (1 file(s))\n"),
            Some("office-123")
        );
        assert_eq!(request_id(""), None);
    }
    #[test]
    fn timeout_reaps_the_service() {
        assert!(
            run_cups("sleep", &["2"], Duration::from_millis(20))
                .unwrap_err()
                .contains("timed out")
        );
    }
    #[test]
    fn failed_service_preserves_the_diagnostic() {
        let output = run_cups(
            "sh",
            &["-c", "echo printer-unavailable >&2; exit 1"],
            Duration::from_secs(2),
        )
        .unwrap();
        assert!(service_error("Printing failed", &output).contains("printer-unavailable"));
    }
}
