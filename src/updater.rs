use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

use crossbeam_channel::Sender;
use serde::{Deserialize, Serialize};

use crate::settings::app_data_dir;

const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/yonathanarbel/LawPDF/releases/latest";
const PORTABLE_ASSET_NAME: &str = "LawPDF-windows-portable-x64.zip";
const INSTALLER_ASSET_NAME: &str = "LawPDFSetup-x64.exe";
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

pub fn updates_enabled() -> bool {
    true
}

pub fn spawn_update_check(tx: Sender<UpdateEvent>) {
    if !updates_enabled() {
        return;
    }

    thread::spawn(move || {
        let _ = tx.send(UpdateEvent::Checking);
        if let Err(error) = check_and_stage_update(&tx) {
            let _ = tx.send(UpdateEvent::Failed(error));
        }
    });
}

pub fn load_pending_update() -> Option<PendingUpdate> {
    if !updates_enabled() {
        return None;
    }

    let path = pending_update_path()?;
    let bytes = std::fs::read(&path).ok()?;
    let pending = serde_json::from_slice::<PendingUpdate>(&bytes).ok()?;
    if !pending.asset_path.exists() || !is_newer_version(&pending.version, CURRENT_VERSION) {
        let _ = std::fs::remove_file(path);
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

    let _ = tx.send(UpdateEvent::Detected {
        version: version.clone(),
    });
    let asset_path = download_asset(tx, &version, asset)?;
    let pending = PendingUpdate {
        version,
        package_kind,
        asset_path,
        release_url: release.html_url,
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
