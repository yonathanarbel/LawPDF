//! Windows spooler bridge. Requests travel as JSON, never executable text.
//! Run on a worker thread: drivers and the native print dialog can block.
use std::path::PathBuf;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Printer {
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SpoolOptions {
    pub printer: String,
    pub copies: u16,
    pub collate: bool,
    /// `simplex`, `long-edge`, or `short-edge`.
    pub duplex: String,
    /// `Letter`, `Legal`, `A4`, or `A3`.
    pub paper: String,
    pub landscape: bool,
    pub system_dialog: bool,
}

pub fn printers() -> Result<Vec<Printer>, String> {
    #[cfg(windows)]
    {
        serde_json::from_value(run_helper(&serde_json::json!({ "action": "printers" }))?)
            .map_err(|error| format!("Windows returned an invalid printer list: {error}"))
    }
    #[cfg(not(windows))]
    {
        Err("Direct printing is currently available on Windows. Open this PDF in your system PDF viewer and use its Print command.".to_owned())
    }
}

/// `true` means Windows accepted the job, not that paper has been printed.
/// `false` means the native dialog was cancelled.
pub fn print_pages(title: &str, pages: &[PathBuf], options: &SpoolOptions) -> Result<bool, String> {
    validate_request(pages, options)?;
    #[cfg(windows)]
    {
        run_helper(&serde_json::json!({
            "action": "print", "title": title, "pages": pages, "options": options,
        }))?
        .as_bool()
        .ok_or_else(|| "Windows returned an invalid print-job result.".to_owned())
    }
    #[cfg(not(windows))]
    {
        let _ = title;
        Err("Direct printing is currently available on Windows. Open this PDF in your system PDF viewer and use its Print command.".to_owned())
    }
}

fn validate_request(pages: &[PathBuf], options: &SpoolOptions) -> Result<(), String> {
    if pages.is_empty() {
        return Err("There are no sheets to print.".to_owned());
    }
    if !(1..=999).contains(&options.copies) {
        return Err("Choose between 1 and 999 copies.".to_owned());
    }
    if !matches!(
        options.duplex.as_str(),
        "simplex" | "long-edge" | "short-edge"
    ) {
        return Err("Choose one-sided, long-edge, or short-edge printing.".to_owned());
    }
    if !matches!(options.paper.as_str(), "Letter" | "Legal" | "A4" | "A3") {
        return Err("Choose Letter, Legal, A4, or A3 paper.".to_owned());
    }
    if pages
        .iter()
        .any(|page| !page.is_absolute() || !page.is_file())
    {
        return Err(
            "A prepared print sheet is missing. Close the print dialog and try again.".to_owned(),
        );
    }
    Ok(())
}

#[cfg(windows)]
fn run_helper(request: &serde_json::Value) -> Result<serde_json::Value, String> {
    run_helper_script(request, POWERSHELL_SCRIPT)
}

#[cfg(windows)]
fn run_helper_script(
    request: &serde_json::Value,
    script: &str,
) -> Result<serde_json::Value, String> {
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    let payload = serde_json::to_vec(request)
        .map_err(|error| format!("Could not prepare the Windows print request: {error}"))?;
    // Bypass PATH so a document directory cannot supply the helper executable.
    let system_root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let mut child =
        Command::new(system_root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe"))
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-STA",
                "-Command",
                script,
            ])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW; native dialogs still work.
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("Could not start Windows printing: {error}"))?;
    let written = match child.stdin.take() {
        Some(mut input) => input.write_all(&payload),
        None => Err(std::io::Error::other(
            "The print helper has no input stream",
        )),
    };
    if let Err(error) = written {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("Could not send the Windows print request: {error}"));
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("Could not wait for Windows printing: {error}"))?;
    #[derive(serde::Deserialize)]
    struct Reply {
        ok: bool,
        #[serde(default)]
        value: serde_json::Value,
        #[serde(default)]
        error: String,
    }
    let reply: Reply = serde_json::from_slice(&output.stdout).map_err(|_| {
        "Windows could not initialize printing. Check that the Print Spooler service and Windows PowerShell are available.".to_owned()
    })?;
    if !reply.ok {
        return Err(if reply.error.is_empty() {
            "Windows could not complete the print request.".to_owned()
        } else {
            reply.error
        });
    }
    if !output.status.success() {
        return Err("The Windows print helper exited unexpectedly.".to_owned());
    }
    Ok(reply.value)
}

#[cfg(windows)]
const POWERSHELL_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::InputEncoding = New-Object System.Text.UTF8Encoding($false)
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$document = $null
$dialog = $null
$handler = $null
try {
    $request = [Console]::In.ReadToEnd() | ConvertFrom-Json
    Add-Type -AssemblyName System.Drawing
    if ($request.action -eq 'printers') {
        $defaultSettings = New-Object System.Drawing.Printing.PrinterSettings
        $defaultName = $defaultSettings.PrinterName
        $result = @([System.Drawing.Printing.PrinterSettings]::InstalledPrinters | ForEach-Object {
            @{ name = [string]$_; is_default = ([string]$_ -eq $defaultName) }
        } | Sort-Object -Property @{Expression = {$_.is_default}; Descending = $true}, @{Expression = {$_.name}})
        [Console]::Out.WriteLine((@{ ok = $true; value = $result } | ConvertTo-Json -Depth 6 -Compress))
        exit 0
    }
    if ($request.action -ne 'print') { throw 'Unknown print operation.' }
    $options = $request.options
    $script:sheetPaths = @($request.pages)
    if ($script:sheetPaths.Count -eq 0) { throw 'There are no sheets to print.' }
    foreach ($path in $script:sheetPaths) {
        if (-not [System.IO.File]::Exists([string]$path)) { throw 'A prepared print sheet is missing. Please prepare the document again.' }
    }
    $document = New-Object System.Drawing.Printing.PrintDocument
    if (-not [string]::IsNullOrWhiteSpace([string]$options.printer)) {
        $document.PrinterSettings.PrinterName = [string]$options.printer
    }
    if (-not $document.PrinterSettings.IsValid) { throw 'The selected printer is unavailable. Choose an installed printer.' }
    $document.DocumentName = [string]$request.title
    $document.OriginAtMargins = $false
    $document.PrintController = New-Object System.Drawing.Printing.StandardPrintController
    $document.DefaultPageSettings.Margins = New-Object System.Drawing.Printing.Margins(0, 0, 0, 0)
    $document.DefaultPageSettings.Landscape = [bool]$options.landscape
    $paper = @($document.PrinterSettings.PaperSizes | Where-Object { $_.Kind.ToString() -eq [string]$options.paper })
    if ($paper.Count -eq 0) { throw ('This printer does not support ' + [string]$options.paper + ' paper. Choose another paper size or printer.') }
    $document.DefaultPageSettings.PaperSize = $paper[0]
    $copies = [int]$options.copies
    if ($copies -lt 1 -or $copies -gt 999) { throw 'Choose between 1 and 999 copies.' }
    if ($copies -gt $document.PrinterSettings.MaximumCopies) { throw ('This printer supports at most ' + $document.PrinterSettings.MaximumCopies + ' copies per job.') }
    $document.PrinterSettings.Copies = [int16]$copies
    $document.PrinterSettings.Collate = [bool]$options.collate
    switch ([string]$options.duplex) {
        'simplex' { $document.PrinterSettings.Duplex = [System.Drawing.Printing.Duplex]::Simplex }
        'long-edge' {
            if (-not $document.PrinterSettings.CanDuplex) { throw 'This printer does not support automatic two-sided printing. Choose one-sided printing.' }
            $document.PrinterSettings.Duplex = [System.Drawing.Printing.Duplex]::Vertical
        }
        'short-edge' {
            if (-not $document.PrinterSettings.CanDuplex) { throw 'This printer does not support automatic two-sided printing. Choose one-sided printing.' }
            $document.PrinterSettings.Duplex = [System.Drawing.Printing.Duplex]::Horizontal
        }
        default { throw 'Unknown two-sided printing setting.' }
    }
    if ([bool]$options.system_dialog) {
        Add-Type -AssemblyName System.Windows.Forms
        $dialog = New-Object System.Windows.Forms.PrintDialog
        $dialog.Document = $document
        $dialog.UseEXDialog = $true
        $dialog.AllowSelection = $false
        $dialog.AllowSomePages = $false
        $dialog.AllowCurrentPage = $false
        $dialog.AllowPrintToFile = $false
        if ($dialog.ShowDialog() -ne [System.Windows.Forms.DialogResult]::OK) {
            [Console]::Out.WriteLine('{"ok":true,"value":false}')
            exit 0
        }
        if (-not $document.PrinterSettings.IsValid) { throw 'The selected printer is unavailable.' }
        $supportedPaper = @($document.PrinterSettings.PaperSizes | Where-Object { $_.Kind.ToString() -eq [string]$options.paper })
        if ($supportedPaper.Count -eq 0) { throw 'The printer selected in Windows does not support the prepared paper size. Choose a supported paper size in LawPDF.' }
        if ($document.DefaultPageSettings.PaperSize.Kind.ToString() -ne [string]$options.paper -or $document.DefaultPageSettings.Landscape -ne [bool]$options.landscape) {
            throw 'The paper size or orientation changed in Windows. Set the same paper and orientation in LawPDF so the preview matches the printed sheets.'
        }
        if ($document.PrinterSettings.Duplex -ne [System.Drawing.Printing.Duplex]::Simplex -and $document.PrinterSettings.Duplex -ne [System.Drawing.Printing.Duplex]::Default -and -not $document.PrinterSettings.CanDuplex) {
            throw 'The selected printer does not support automatic two-sided printing. Choose one-sided printing.'
        }
    }
    $script:sheetIndex = 0
    $script:printError = $null
    $handler = [System.Drawing.Printing.PrintPageEventHandler]{
        param($sender, $eventArgs)
        $image = $null
        try {
            $image = [System.Drawing.Image]::FromFile([string]$script:sheetPaths[$script:sheetIndex])
            $graphics = $eventArgs.Graphics
            $graphics.PageUnit = [System.Drawing.GraphicsUnit]::Display
            $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
            # The PNG describes physical paper. GDI begins at the printable
            # corner, so shift back and let the driver clip unprintable areas.
            $graphics.TranslateTransform(-[single]$eventArgs.PageSettings.HardMarginX, -[single]$eventArgs.PageSettings.HardMarginY)
            $bounds = New-Object System.Drawing.RectangleF(0, 0, [single]$eventArgs.PageBounds.Width, [single]$eventArgs.PageBounds.Height)
            $graphics.DrawImage($image, $bounds)
            $script:sheetIndex++
            $eventArgs.HasMorePages = ($script:sheetIndex -lt $script:sheetPaths.Count)
        } catch {
            $script:printError = $_.Exception.Message
            $eventArgs.Cancel = $true
            $eventArgs.HasMorePages = $false
        } finally {
            if ($null -ne $image) { $image.Dispose() }
        }
    }
    $document.add_PrintPage($handler)
    $document.Print()
    if ($null -ne $script:printError) { throw $script:printError }
    [Console]::Out.WriteLine('{"ok":true,"value":true}')
} catch {
    [Console]::Out.WriteLine((@{ ok = $false; error = $_.Exception.Message } | ConvertTo-Json -Depth 4 -Compress))
    exit 1
} finally {
    if ($null -ne $dialog) { $dialog.Dispose() }
    if ($null -ne $document) {
        if ($null -ne $handler) { $document.remove_PrintPage($handler) }
        $document.Dispose()
    }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    fn options() -> SpoolOptions {
        SpoolOptions {
            printer: "Printer 'with' $special `characters`".to_owned(),
            copies: 1,
            collate: true,
            duplex: "simplex".to_owned(),
            paper: "Letter".to_owned(),
            landscape: false,
            system_dialog: false,
        }
    }
    #[test]
    fn rejects_invalid_requests_without_starting_a_helper() {
        let sheet = tempfile::NamedTempFile::new().unwrap();
        let pages = vec![sheet.path().to_path_buf()];
        assert!(validate_request(&pages, &options()).is_ok());
        assert!(validate_request(&[], &options()).is_err());
        let mut invalid = options();
        invalid.copies = 0;
        assert!(validate_request(&pages, &invalid).is_err());
        invalid = options();
        invalid.duplex = "automatic".to_owned();
        assert!(validate_request(&pages, &invalid).is_err());
        invalid = options();
        invalid.paper = "unknown".to_owned();
        assert!(validate_request(&pages, &invalid).is_err());
        assert!(validate_request(&[PathBuf::from("missing-sheet.png")], &options()).is_err());
    }
    #[test]
    fn printer_names_are_json_data() {
        let expected = options();
        let encoded = serde_json::to_value(&expected).unwrap();
        assert_eq!(encoded["printer"], expected.printer);
        assert_eq!(encoded["system_dialog"], false);
    }

    /// Uses printer metrics, but PreviewPrintController creates only metafiles.
    /// This must never fall back to StandardPrintController or a real print job.
    #[cfg(windows)]
    #[test]
    #[ignore = "Requires Windows PowerShell and Microsoft Print to PDF; creates previews only"]
    fn windows_preview_controller_renders_without_spooling() {
        const STANDARD: &str = "System.Drawing.Printing.StandardPrintController";
        assert_eq!(POWERSHELL_SCRIPT.matches(STANDARD).count(), 1);
        assert_eq!(POWERSHELL_SCRIPT.matches("$document.Print()").count(), 1);
        let script =
            POWERSHELL_SCRIPT.replace(STANDARD, "System.Drawing.Printing.PreviewPrintController");
        let script = script.replace("    $document.Print()", r#"
    $document.Print()
    if ($null -ne $script:printError) { throw $script:printError }
    $previews = $document.PrintController.GetPreviewPageInfo()
    if ($previews.Count -ne 2) { throw 'Expected two preview pages.' }
    $previewIndex = 0
    foreach ($preview in $previews) {
        $bitmap = New-Object System.Drawing.Bitmap($preview.PhysicalSize.Width, $preview.PhysicalSize.Height)
        $canvas = [System.Drawing.Graphics]::FromImage($bitmap)
        try {
            $canvas.Clear([System.Drawing.Color]::White)
            $canvas.DrawImage($preview.Image, 0, 0, $bitmap.Width, $bitmap.Height)
            $destination = [System.IO.Path]::Combine([string]$request.preview_directory, ('preview-' + $previewIndex + '.png'))
            $bitmap.Save($destination, [System.Drawing.Imaging.ImageFormat]::Png)
        } finally {
            $canvas.Dispose()
            $bitmap.Dispose()
            $preview.Image.Dispose()
        }
        $previewIndex++
    }
"#);
        assert!(!script.contains("StandardPrintController"));
        assert!(script.contains(
            "$document.PrintController = New-Object System.Drawing.Printing.PreviewPrintController"
        ));
        assert!(
            printers()
                .unwrap()
                .iter()
                .any(|printer| printer.name == "Microsoft Print to PDF")
        );

        for landscape in [false, true] {
            let directory = tempfile::Builder::new()
                .prefix("lawpdf-print-backend-qa-")
                .tempdir()
                .unwrap();
            let (width, height) = if landscape { (1100, 850) } else { (850, 1100) };
            let colors = [[220, 25, 35, 255], [30, 180, 55, 255]];
            let mut pages = Vec::new();
            for (index, color) in colors.iter().enumerate() {
                let mut image = image::RgbaImage::from_pixel(width, height, image::Rgba([255; 4]));
                for x in 100..200 {
                    for y in 100..200 {
                        image.put_pixel(x, y, image::Rgba(*color));
                    }
                }
                for x in width - 200..width - 100 {
                    for y in height - 200..height - 100 {
                        image.put_pixel(x, y, image::Rgba([25, 45, 220, 255]));
                    }
                }
                // Names with shell metacharacters also exercise the JSON boundary.
                let path = directory.path().join(format!("sheet-{index}-$`-ü.png"));
                image.save(&path).unwrap();
                pages.push(path);
            }
            let mut spool = options();
            spool.printer = "Microsoft Print to PDF".to_owned();
            spool.landscape = landscape;
            let request = serde_json::json!({
                "action": "print", "title": "LawPDF 'preview' $` ü - no spooling", "pages": pages,
                "options": spool, "preview_directory": directory.path(),
            });
            assert_eq!(run_helper_script(&request, &script).unwrap(), true);
            for (index, expected) in colors.iter().enumerate() {
                let preview = image::open(directory.path().join(format!("preview-{index}.png")))
                    .unwrap()
                    .to_rgba8();
                assert_eq!(preview.dimensions(), (width, height));
                let marker = preview.get_pixel(150, 150);
                for channel in 0..3 {
                    assert!(
                        (i16::from(marker[channel]) - i16::from(expected[channel])).abs() <= 5,
                        "Wrong page, orientation, or physical marker position: {marker:?}"
                    );
                }
                let far_marker = preview.get_pixel(width - 150, height - 150);
                assert!(far_marker[2] > 200 && far_marker[0] < 40);
                assert_eq!(*preview.get_pixel(50, 50), image::Rgba([255; 4]));
            }
            // Exercise native capability errors using the same safe controller.
            let mut unsupported = request.clone();
            unsupported["options"]["duplex"] = "long-edge".into();
            assert!(
                run_helper_script(&unsupported, &script)
                    .unwrap_err()
                    .contains("does not support automatic two-sided")
            );
            unsupported["options"]["duplex"] = "simplex".into();
            unsupported["options"]["paper"] = "LawPDF unsupported paper".into();
            assert!(
                run_helper_script(&unsupported, &script)
                    .unwrap_err()
                    .contains("does not support LawPDF unsupported paper")
            );
        }
    }
}
