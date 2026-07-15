use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use crossbeam_channel::Sender;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::settings::app_data_dir;

const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/yonathanarbel/LawPDF/releases/latest";
const PORTABLE_ASSET_NAME: &str = "LawPDF-windows-portable-x64.zip";
const INSTALLER_ASSET_NAME: &str = "LawPDFSetup-x64.exe";
const SHA256SUMS_ASSET_NAME: &str = "SHA256SUMS.txt";
const USER_AGENT: &str = concat!("LawPDF/", env!("CARGO_PKG_VERSION"));
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone)]
pub enum UpdateEvent {
    Checking,
    Detected { version: String },
    NotAvailable,
    Downloading,
    Ready(PendingUpdate),
    Failed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingUpdate {
    pub version: String,
    pub package_kind: UpdatePackageKind,
    pub asset_path: PathBuf,
    pub release_url: String,
    #[serde(default)]
    pub expected_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdatePackageKind {
    PortableZip,
    Installer,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    prerelease: bool,
    draft: bool,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

pub fn spawn_update_check(tx: Sender<UpdateEvent>) {
    thread::spawn(move || {
        let _ = tx.send(UpdateEvent::Checking);
        if let Err(error) = check_and_stage_update(&tx) {
            let _ = tx.send(UpdateEvent::Failed(error));
        }
    });
}

pub fn load_pending_update() -> Option<PendingUpdate> {
    let path = pending_update_path()?;
    let bytes = std::fs::read(&path).ok()?;
    let pending = match serde_json::from_slice::<PendingUpdate>(&bytes) {
        Ok(pending) => pending,
        Err(_) => {
            let _ = std::fs::remove_file(path);
            return None;
        }
    };
    if !pending.asset_path.exists() || !is_newer_version(&pending.version, CURRENT_VERSION) {
        let _ = std::fs::remove_file(path);
        return None;
    }
    if verify_pending_update(&pending).is_err() {
        discard_pending_update(&pending, &path);
        return None;
    }
    Some(pending)
}

pub fn take_installed_update() -> Option<String> {
    let path = installed_update_path()?;
    let version = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(path);
    let version = normalize_version(&version);
    if version.is_empty() || version_numbers(&version) > version_numbers(CURRENT_VERSION) {
        return None;
    }
    Some(version)
}

pub fn start_update_helper(
    pending: &PendingUpdate,
    relaunch_args: &[OsString],
) -> Result<(), String> {
    if let Err(error) = verify_pending_update(pending) {
        if let Some(path) = pending_update_path() {
            discard_pending_update(pending, &path);
        }
        return Err(error);
    }
    let script_path = write_update_script(pending, relaunch_args)?;
    let mut command = Command::new("powershell.exe");
    command
        .arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-File")
        .arg(script_path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not start the update helper: {error}"))
}

fn check_and_stage_update(tx: &Sender<UpdateEvent>) -> Result<(), String> {
    if let Some(pending) = load_pending_update() {
        let _ = tx.send(UpdateEvent::Ready(pending));
        return Ok(());
    }

    let release = fetch_latest_release()?;
    if release.draft || release.prerelease {
        let _ = tx.send(UpdateEvent::NotAvailable);
        return Ok(());
    }

    let version = normalize_version(&release.tag_name);
    if !is_newer_version(&version, CURRENT_VERSION) {
        let _ = tx.send(UpdateEvent::NotAvailable);
        return Ok(());
    }

    let package_kind = preferred_package_kind();
    let asset_name = match package_kind {
        UpdatePackageKind::PortableZip => PORTABLE_ASSET_NAME,
        UpdatePackageKind::Installer => INSTALLER_ASSET_NAME,
    };
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name.eq_ignore_ascii_case(asset_name))
        .ok_or_else(|| format!("Release {version} does not include {asset_name}."))?;
    let Some(checksums_asset) = release
        .assets
        .iter()
        .find(|asset| asset.name.eq_ignore_ascii_case(SHA256SUMS_ASSET_NAME))
    else {
        let _ = tx.send(UpdateEvent::Failed(
            "Release is missing SHA256SUMS.txt; not updating.".to_owned(),
        ));
        return Ok(());
    };
    let checksums = download_text_asset(checksums_asset)?;
    let expected_sha256 = parse_sha256sums(&checksums)
        .into_iter()
        .find(|(_, name)| name.eq_ignore_ascii_case(&asset.name))
        .map(|(hash, _)| hash)
        .ok_or_else(|| {
            format!(
                "SHA256SUMS.txt does not contain a checksum for {}.",
                asset.name
            )
        })?;
    if expected_sha256.len() != 64 || !expected_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!(
            "SHA256SUMS.txt contains an invalid checksum for {}.",
            asset.name
        ));
    }

    let _ = tx.send(UpdateEvent::Detected {
        version: version.clone(),
    });
    let asset_path = download_asset(tx, &version, asset)?;
    let actual_sha256 = match sha256_hex_of_file(&asset_path) {
        Ok(hash) => hash,
        Err(error) => {
            let _ = std::fs::remove_file(&asset_path);
            return Err(error);
        }
    };
    if !actual_sha256.eq_ignore_ascii_case(&expected_sha256) {
        let _ = std::fs::remove_file(&asset_path);
        let _ = tx.send(UpdateEvent::Failed(format!(
            "Update checksum mismatch; expected {expected_sha256}, downloaded {actual_sha256}. The package was discarded."
        )));
        return Ok(());
    }
    let pending = PendingUpdate {
        version,
        package_kind,
        asset_path,
        release_url: release.html_url,
        expected_sha256: expected_sha256.to_ascii_lowercase(),
    };
    write_pending_update(&pending)?;
    let _ = tx.send(UpdateEvent::Ready(pending));
    Ok(())
}

fn fetch_latest_release() -> Result<GithubRelease, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| format!("Could not create update client: {error}"))?
        .get(GITHUB_LATEST_RELEASE_URL)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| format!("Could not check for updates: {error}"))?
        .json::<GithubRelease>()
        .map_err(|error| format!("Could not read update metadata: {error}"))
}

fn download_text_asset(asset: &GithubAsset) -> Result<String, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| format!("Could not create checksum downloader: {error}"))?
        .get(&asset.browser_download_url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| format!("Could not download SHA256SUMS.txt: {error}"))?
        .text()
        .map_err(|error| format!("Could not read SHA256SUMS.txt: {error}"))
}

fn download_asset(
    tx: &Sender<UpdateEvent>,
    version: &str,
    asset: &GithubAsset,
) -> Result<PathBuf, String> {
    let update_dir = updates_dir()
        .ok_or_else(|| "Could not find a writable update directory.".to_owned())?
        .join(version);
    std::fs::create_dir_all(&update_dir)
        .map_err(|error| format!("Could not create update folder: {error}"))?;
    let asset_path = update_dir.join(&asset.name);
    let partial_path = update_dir.join(format!("{}.download", asset.name));

    let mut response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|error| format!("Could not create update downloader: {error}"))?
        .get(&asset.browser_download_url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| format!("Could not download update: {error}"))?;

    let mut file = std::fs::File::create(&partial_path)
        .map_err(|error| format!("Could not create update package: {error}"))?;
    let mut buffer = [0_u8; 64 * 1024];

    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|error| format!("Could not read update package: {error}"))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|error| format!("Could not save update package: {error}"))?;
        let _ = tx.send(UpdateEvent::Downloading);
    }

    std::fs::rename(&partial_path, &asset_path)
        .map_err(|error| format!("Could not stage update package: {error}"))?;
    Ok(asset_path)
}

fn parse_sha256sums(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let hash_end = line.find(char::is_whitespace)?;
            let hash = line[..hash_end].trim();
            let name = line[hash_end..]
                .trim_start()
                .strip_prefix('*')
                .unwrap_or_else(|| line[hash_end..].trim_start())
                .trim();
            (!hash.is_empty() && !name.is_empty()).then(|| (hash.to_owned(), name.to_owned()))
        })
        .collect()
}

pub(crate) fn sha256_hex_of_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|error| {
        format!(
            "Could not open {} for verification: {error}",
            path.display()
        )
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("Could not verify {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn verify_pending_update(pending: &PendingUpdate) -> Result<(), String> {
    let expected = pending.expected_sha256.trim();
    if expected.is_empty() {
        return Err("Staged update has no trusted SHA-256 checksum; it was discarded.".to_owned());
    }
    let actual = sha256_hex_of_file(&pending.asset_path)?;
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(format!(
            "Staged update checksum mismatch; expected {expected}, found {actual}. It was discarded."
        ));
    }
    Ok(())
}

fn discard_pending_update(pending: &PendingUpdate, manifest_path: &Path) {
    if updates_dir().is_some_and(|dir| pending.asset_path.starts_with(dir)) {
        let _ = std::fs::remove_file(&pending.asset_path);
    }
    let _ = std::fs::remove_file(manifest_path);
}

fn preferred_package_kind() -> UpdatePackageKind {
    if let Ok(exe) = std::env::current_exe() {
        if exe
            .parent()
            .is_some_and(|dir| dir.join("unins000.exe").exists())
        {
            return UpdatePackageKind::Installer;
        }
    }
    UpdatePackageKind::PortableZip
}

fn write_pending_update(pending: &PendingUpdate) -> Result<(), String> {
    let path =
        pending_update_path().ok_or_else(|| "Could not find update state path.".to_owned())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create update state folder: {error}"))?;
    }
    let bytes = serde_json::to_vec_pretty(pending)
        .map_err(|error| format!("Could not encode update state: {error}"))?;
    std::fs::write(path, bytes).map_err(|error| format!("Could not save update state: {error}"))
}

fn pending_update_path() -> Option<PathBuf> {
    updates_dir().map(|path| path.join("pending_update.json"))
}

fn installed_update_path() -> Option<PathBuf> {
    updates_dir().map(|path| path.join("installed_update.txt"))
}

fn updates_dir() -> Option<PathBuf> {
    app_data_dir().map(|path| path.join("updates"))
}

fn write_update_script(
    pending: &PendingUpdate,
    relaunch_args: &[OsString],
) -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|error| format!("Could not locate the running executable: {error}"))?;
    let app_dir = exe
        .parent()
        .ok_or_else(|| "Could not locate the application folder.".to_owned())?;
    let updates_dir =
        updates_dir().ok_or_else(|| "Could not locate the update folder.".to_owned())?;
    std::fs::create_dir_all(&updates_dir)
        .map_err(|error| format!("Could not create update helper folder: {error}"))?;

    let script_path = updates_dir.join("finish-update.ps1");
    let manifest_path =
        pending_update_path().ok_or_else(|| "Could not locate update state.".to_owned())?;
    let installed_path = installed_update_path()
        .ok_or_else(|| "Could not locate installed update state.".to_owned())?;
    let error_path = updates_dir.join("last-update-error.txt");
    let extract_dir = updates_dir.join(format!("extract-{}", pending.version));
    let relaunch_args = powershell_argument_list(relaunch_args);
    let installed_version = ps_literal(&pending.version);
    let script = match pending.package_kind {
        UpdatePackageKind::PortableZip => format!(
            r#"$ErrorActionPreference = 'Stop'
$pidToWait = {pid}
$archive = {archive}
$appDir = {app_dir}
$exe = {exe}
$manifest = {manifest}
$installedPath = {installed_path}
$installedVersion = {installed_version}
$errorPath = {error_path}
$extractDir = {extract_dir}
$relaunchArgs = {relaunch_args}
try {{
    Wait-Process -Id $pidToWait -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath $extractDir) {{
        Remove-Item -LiteralPath $extractDir -Recurse -Force
    }}
    New-Item -ItemType Directory -Force -Path $extractDir | Out-Null
    Expand-Archive -LiteralPath $archive -DestinationPath $extractDir -Force
    Get-ChildItem -LiteralPath $extractDir -Force | Copy-Item -Destination $appDir -Recurse -Force
    Remove-Item -LiteralPath $manifest -Force -ErrorAction SilentlyContinue
    Set-Content -LiteralPath $installedPath -Value $installedVersion -Encoding UTF8
    Remove-Item -LiteralPath $archive -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $extractDir -Recurse -Force -ErrorAction SilentlyContinue
}} catch {{
    $_ | Out-String | Set-Content -LiteralPath $errorPath -Encoding UTF8
    Remove-Item -LiteralPath $manifest -Force -ErrorAction SilentlyContinue
}}
Start-Process -FilePath $exe -ArgumentList $relaunchArgs -WorkingDirectory $appDir
"#,
            pid = std::process::id(),
            archive = ps_string(&pending.asset_path),
            app_dir = ps_string(app_dir),
            exe = ps_string(&exe),
            manifest = ps_string(&manifest_path),
            installed_path = ps_string(&installed_path),
            installed_version = installed_version,
            error_path = ps_string(&error_path),
            extract_dir = ps_string(&extract_dir),
            relaunch_args = relaunch_args,
        ),
        UpdatePackageKind::Installer => format!(
            r#"$ErrorActionPreference = 'Stop'
$pidToWait = {pid}
$installer = {installer}
$appDir = {app_dir}
$exe = {exe}
$manifest = {manifest}
$installedPath = {installed_path}
$installedVersion = {installed_version}
$errorPath = {error_path}
$relaunchArgs = {relaunch_args}
try {{
    Wait-Process -Id $pidToWait -ErrorAction SilentlyContinue
    $installerArgs = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CLOSEAPPLICATIONS', '/RESTARTAPPLICATIONS')
    $process = Start-Process -FilePath $installer -ArgumentList $installerArgs -Wait -PassThru
    if ($process.ExitCode -ne 0) {{
        throw "Installer exited with code $($process.ExitCode)."
    }}
    Remove-Item -LiteralPath $manifest -Force -ErrorAction SilentlyContinue
    Set-Content -LiteralPath $installedPath -Value $installedVersion -Encoding UTF8
    Remove-Item -LiteralPath $installer -Force -ErrorAction SilentlyContinue
}} catch {{
    $_ | Out-String | Set-Content -LiteralPath $errorPath -Encoding UTF8
    Remove-Item -LiteralPath $manifest -Force -ErrorAction SilentlyContinue
}}
Start-Process -FilePath $exe -ArgumentList $relaunchArgs -WorkingDirectory $appDir
"#,
            pid = std::process::id(),
            installer = ps_string(&pending.asset_path),
            app_dir = ps_string(app_dir),
            exe = ps_string(&exe),
            manifest = ps_string(&manifest_path),
            installed_path = ps_string(&installed_path),
            installed_version = installed_version,
            error_path = ps_string(&error_path),
            relaunch_args = relaunch_args,
        ),
    };

    std::fs::write(&script_path, script)
        .map_err(|error| format!("Could not write update helper: {error}"))?;
    Ok(script_path)
}

fn ps_string(path: impl AsRef<std::path::Path>) -> String {
    let value = path.as_ref().as_os_str().to_string_lossy();
    ps_literal(&value)
}

fn ps_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn powershell_argument_list(args: &[OsString]) -> String {
    let value = args
        .iter()
        .map(|arg| windows_command_line_arg(&arg.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ");
    ps_literal(&value)
}

fn windows_command_line_arg(arg: &str) -> String {
    if !arg.is_empty()
        && !arg
            .chars()
            .any(|ch| ch.is_whitespace() || ch == '"' || ch == '\\')
    {
        return arg.to_owned();
    }

    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.push_str(&"\\".repeat(backslashes));
                quoted.push(ch);
                backslashes = 0;
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

fn normalize_version(version: &str) -> String {
    version
        .trim()
        .trim_start_matches('v')
        .trim_start_matches('V')
        .to_owned()
}

fn is_newer_version(candidate: &str, current: &str) -> bool {
    version_numbers(candidate) > version_numbers(current)
}

fn version_numbers(version: &str) -> Vec<u64> {
    version
        .trim()
        .trim_start_matches('v')
        .trim_start_matches('V')
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parses_standard_sha256sums_format() {
        let parsed = parse_sha256sums(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  LawPDFSetup-x64.exe\n\
             bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb LawPDF-macos.zip\n",
        );
        assert_eq!(
            parsed,
            vec![
                (
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
                    "LawPDFSetup-x64.exe".to_owned()
                ),
                (
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned(),
                    "LawPDF-macos.zip".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn parses_star_prefixed_names_and_crlf() {
        let parsed = parse_sha256sums(
            "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC *LawPDF-windows-portable-x64.zip\r\n",
        );
        assert_eq!(
            parsed,
            vec![(
                "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC".to_owned(),
                "LawPDF-windows-portable-x64.zip".to_owned()
            )]
        );
    }

    #[test]
    fn missing_sha256_entry_stays_missing() {
        let parsed = parse_sha256sums(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  other.zip\n",
        );
        assert!(
            parsed
                .iter()
                .all(|(_, name)| !name.eq_ignore_ascii_case(INSTALLER_ASSET_NAME))
        );
    }

    #[test]
    fn hashes_file_with_known_content() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "lawpdf-sha256-test-{}-{unique}.txt",
            std::process::id()
        ));
        std::fs::write(&path, b"abc").expect("write hash fixture");
        let hash = sha256_hex_of_file(&path).expect("hash fixture");
        let _ = std::fs::remove_file(path);
        assert_eq!(
            hash,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn compares_release_versions_and_rejects_garbage() {
        assert!(!is_newer_version("v0.2.6", "0.2.6"));
        assert!(is_newer_version("0.2.10", "0.2.9"));
        assert!(is_newer_version("0.3", "0.2.6"));
        assert!(!is_newer_version("garbage", "0.2.6"));
        assert!(!is_newer_version("garbage", "also-garbage"));
    }
}
