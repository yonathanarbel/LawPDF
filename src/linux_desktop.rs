//! Desktop integration for Linux: the `.desktop` entry that makes LawPDF a
//! choosable PDF application, and the XDG MIME default that points `.pdf` at
//! it.
//!
//! Everything is written under the user's own data directory. LawPDF never
//! edits system-wide association files, so making it the default needs no
//! elevation and a packaged install can ship its own entry under `/usr/share`
//! without the two fighting.
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The desktop-entry file name, which is also the identifier `xdg-mime` uses.
pub const DESKTOP_ENTRY_ID: &str = "lawpdf.desktop";
const ICON_NAME: &str = "lawpdf";
const PDF_MIME_TYPE: &str = "application/pdf";

fn data_home() -> Result<PathBuf, String> {
    data_home_from(std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"))
}

fn data_home_from(
    xdg: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Result<PathBuf, String> {
    if let Some(dir) = xdg.filter(|value| !value.is_empty()) {
        let path = PathBuf::from(dir);
        if path.is_absolute() {
            return Ok(path);
        }
    }
    let home = home
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "No home directory is set, so LawPDF cannot register itself.".to_owned())?;
    let home = PathBuf::from(home);
    if !home.is_absolute() {
        return Err("The home directory must be an absolute path.".to_owned());
    }
    Ok(home.join(".local").join("share"))
}

fn applications_dir() -> Result<PathBuf, String> {
    Ok(data_home()?.join("applications"))
}

fn icon_path() -> Result<PathBuf, String> {
    Ok(data_home()?
        .join("icons")
        .join("hicolor")
        .join("256x256")
        .join("apps")
        .join(format!("{ICON_NAME}.png")))
}

/// The executable the desktop entry should launch. A packaged install runs from
/// a stable location; a development build points at the build output, which is
/// still correct for the machine it was registered on.
fn executable() -> Result<PathBuf, String> {
    let path = std::env::current_exe()
        .map_err(|error| format!("Could not locate the LawPDF executable: {error}"))?;
    path.canonicalize().or(Ok(path))
}

/// Escape both desktop-entry string parsing and Exec argument parsing. In
/// particular, a literal backslash needs four backslashes in the file, and a
/// percent sign must not become a field code. No shell executes this value.
fn exec_quote(path: &str) -> String {
    let mut escaped = String::from("\"");
    for character in path.chars() {
        match character {
            '\\' => escaped.push_str(r"\\\\"),
            '"' => escaped.push_str(r#"\\""#),
            '$' => escaped.push_str(r"\\$"),
            '`' => escaped.push_str(r"\\`"),
            '%' => escaped.push_str("%%"),
            _ => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
}

fn desktop_entry_contents(executable: &Path) -> Result<String, String> {
    let path = executable
        .to_str()
        .ok_or_else(|| "Desktop launchers require a UTF-8 executable path.".to_owned())?;
    if path.chars().any(char::is_control) || path.contains('=') {
        return Err("Move LawPDF to a folder whose path has no control characters or equal signs before adding it to the application menu.".to_owned());
    }
    let try_exec = path.replace('\\', r"\\");
    Ok(format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name=LawPDF\n\
         GenericName=PDF Reader\n\
         Comment=Read, annotate, and review PDFs built for legal scholarship\n\
         Exec={exec} %f\n\
         TryExec={try_exec}\n\
         Icon={icon}\n\
         Terminal=false\n\
         Categories=Office;Viewer;Publishing;\n\
         MimeType={mime};\n\
         StartupNotify=true\n\
         StartupWMClass=lawpdf\n",
        exec = exec_quote(path),
        icon = ICON_NAME,
        mime = PDF_MIME_TYPE,
    ))
}

/// Write the desktop entry and icon into the user's data directory, replacing
/// any earlier copy so a moved or rebuilt executable is picked up.
pub fn install_desktop_entry() -> Result<PathBuf, String> {
    let executable = executable()?;
    let applications = applications_dir()?;
    fs::create_dir_all(&applications)
        .map_err(|error| format!("Could not create {}: {error}", applications.display()))?;
    let entry_path = applications.join(DESKTOP_ENTRY_ID);
    let entry = desktop_entry_contents(&executable)?;

    let icon_path = icon_path()?;
    if let Some(parent) = icon_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    }
    write_atomically(&icon_path, include_bytes!("../assets/lawpdf.png"))?;
    write_atomically(&entry_path, entry.as_bytes())?;

    // Best effort: the entry is already valid without a refreshed cache, and
    // these tools are missing on minimal systems.
    run_quietly("update-desktop-database", &[applications.as_os_str()]);
    if let Ok(icons) = data_home().map(|root| root.join("icons").join("hicolor")) {
        run_quietly("gtk-update-icon-cache", &["-q".as_ref(), icons.as_os_str()]);
    }
    Ok(entry_path)
}

/// Remove only a launcher registered by this exact extracted installation.
/// A newer installation may have replaced it since this one was unpacked.
pub fn uninstall_desktop_entry() -> Result<(), String> {
    let entry_path = applications_dir()?.join(DESKTOP_ENTRY_ID);
    let expected = desktop_entry_contents(&executable()?)?;
    let existing = match fs::read_to_string(&entry_path) {
        Ok(existing) => existing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("Could not read {}: {error}", entry_path.display())),
    };
    if existing != expected {
        return Err("The LawPDF menu entry belongs to another installation. Use that installation's uninstall.sh.".to_owned());
    }
    fs::remove_file(&entry_path)
        .map_err(|error| format!("Could not remove {}: {error}", entry_path.display()))?;
    if let Ok(icon) = icon_path() {
        let _ = fs::remove_file(icon);
    }
    if let Some(parent) = entry_path.parent() {
        run_quietly("update-desktop-database", &[parent.as_os_str()]);
    }
    Ok(())
}

/// Register LawPDF as the handler for `application/pdf` for this user.
pub fn set_default_pdf_reader() -> Result<(), String> {
    install_desktop_entry()?;
    let status = Command::new("xdg-mime")
        .args(["default", DESKTOP_ENTRY_ID, PDF_MIME_TYPE])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| {
            format!("Could not run xdg-mime; install xdg-utils to set the default: {error}")
        })?;
    if !status.success() {
        return Err(format!("xdg-mime exited with {status}"));
    }
    // Some desktops accept the command but keep their own association. Report
    // success only when the change is observable, so the status line does not
    // promise something that did not happen.
    if !is_default_pdf_reader() {
        let current = default_pdf_reader().unwrap_or_else(|| "another application".to_owned());
        return Err(format!(
            "your desktop still opens PDFs with {current}; set it from the system settings instead"
        ));
    }
    Ok(())
}

/// The desktop entry currently registered for PDFs, if any.
pub fn default_pdf_reader() -> Option<String> {
    let output = Command::new("xdg-mime")
        .args(["query", "default", PDF_MIME_TYPE])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!value.is_empty()).then_some(value)
}

/// Whether LawPDF already owns the PDF association.
pub fn is_default_pdf_reader() -> bool {
    default_pdf_reader().is_some_and(|entry| entry == DESKTOP_ENTRY_ID)
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    crate::atomic_file::replace_with(path, |file| file.write_all(bytes))
        .map_err(|error| format!("Could not write {}: {error}", path.display()))
}

fn run_quietly(program: &str, args: &[&std::ffi::OsStr]) {
    let _ = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_entry_declares_the_pdf_association() {
        let entry = desktop_entry_contents(Path::new("/usr/bin/lawpdf")).unwrap();
        assert!(entry.starts_with("[Desktop Entry]\n"));
        assert!(entry.contains("MimeType=application/pdf;"));
        assert!(entry.contains("Exec=\"/usr/bin/lawpdf\" %f"));
        assert!(entry.contains("StartupWMClass=lawpdf"));
    }

    #[test]
    fn desktop_path_cannot_inject_entries_or_field_codes() {
        assert!(desktop_entry_contents(Path::new("/opt/bad\nExec=other/lawpdf")).is_err());
        assert!(desktop_entry_contents(Path::new("/opt/a=b/lawpdf")).is_err());
        let entry = desktop_entry_contents(Path::new("/opt/100% ready/lawpdf")).unwrap();
        assert!(entry.contains("Exec=\"/opt/100%% ready/lawpdf\" %f"));
        assert_eq!(exec_quote(r#"/opt/a\b"$`"#), r#""/opt/a\\\\b\\"\\$\\`""#);
    }

    #[test]
    fn xdg_paths_must_be_absolute() {
        assert_eq!(
            data_home_from(Some("relative/path".into()), Some("/home/example".into())).unwrap(),
            PathBuf::from("/home/example/.local/share")
        );
        assert_eq!(
            data_home_from(Some("/custom/share".into()), None).unwrap(),
            PathBuf::from("/custom/share")
        );
        assert!(data_home_from(None, Some("relative".into())).is_err());
    }
}
