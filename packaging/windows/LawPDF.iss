#define MyAppName "LawPDF"
#define MyAppPublisher "Y. Arbel design (2026)"
#define MyAppExeName "lawpdf.exe"
#define MyAppIconName "lawpdf.ico"
#ifndef AppVersion
#define AppVersion "0.1.0"
#endif

[Setup]
AppId={{B76759BB-B39A-4F51-8A3D-EC9A6BB4E5D4}
AppName={#MyAppName}
AppVersion={#AppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
LicenseFile=..\..\LICENSE
SetupIconFile=..\..\assets\lawpdf.ico
OutputDir=..\..\dist
OutputBaseFilename=LawPDFSetup-x64
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\{#MyAppIconName}
ChangesAssociations=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional icons:"; Flags: unchecked

[Files]
Source: "..\..\dist\LawPDF-portable\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Registry]
Root: HKLM; Subkey: "Software\Classes\.pdf\OpenWithProgids"; ValueType: string; ValueName: "LawPDF.PDF"; ValueData: "{#MyAppName}"; Flags: uninsdeletevalue
Root: HKCR; Subkey: "LawPDF.PDF"; ValueType: string; ValueData: "PDF Document"; Flags: uninsdeletekey
Root: HKCR; Subkey: "LawPDF.PDF\DefaultIcon"; ValueType: string; ValueData: "{app}\{#MyAppIconName}"
Root: HKCR; Subkey: "LawPDF.PDF\shell\open\command"; ValueType: string; ValueData: """{app}\{#MyAppExeName}"" ""%1"""
Root: HKLM; Subkey: "Software\{#MyAppName}\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "{#MyAppName}"; Flags: uninsdeletekey
Root: HKLM; Subkey: "Software\{#MyAppName}\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "Read, search, sign, and annotate PDF documents with LawPDF."
Root: HKLM; Subkey: "Software\{#MyAppName}\Capabilities\FileAssociations"; ValueType: string; ValueName: ".pdf"; ValueData: "LawPDF.PDF"
Root: HKLM; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "{#MyAppName}"; ValueData: "Software\{#MyAppName}\Capabilities"; Flags: uninsdeletevalue
Root: HKLM; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\{#MyAppExeName}"; ValueType: string; ValueData: "{app}\{#MyAppExeName}"; Flags: uninsdeletekey
Root: HKLM; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\{#MyAppExeName}"; ValueType: string; ValueName: "Path"; ValueData: "{app}"
Root: HKCR; Subkey: "SystemFileAssociations\.docx\shell\LawPDF.ConvertToPdf"; ValueType: string; ValueData: "Convert to PDF with {#MyAppName}"; Flags: uninsdeletekey
Root: HKCR; Subkey: "SystemFileAssociations\.docx\shell\LawPDF.ConvertToPdf"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#MyAppIconName}"
Root: HKCR; Subkey: "SystemFileAssociations\.docx\shell\LawPDF.ConvertToPdf\command"; ValueType: string; ValueData: """{app}\{#MyAppExeName}"" --convert-to-pdf ""%1"""
Root: HKCR; Subkey: "SystemFileAssociations\.md\shell\LawPDF.ConvertToPdf"; ValueType: string; ValueData: "Convert to PDF with {#MyAppName}"; Flags: uninsdeletekey
Root: HKCR; Subkey: "SystemFileAssociations\.md\shell\LawPDF.ConvertToPdf"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#MyAppIconName}"
Root: HKCR; Subkey: "SystemFileAssociations\.md\shell\LawPDF.ConvertToPdf\command"; ValueType: string; ValueData: """{app}\{#MyAppExeName}"" --convert-to-pdf ""%1"""
Root: HKCR; Subkey: "SystemFileAssociations\.markdown\shell\LawPDF.ConvertToPdf"; ValueType: string; ValueData: "Convert to PDF with {#MyAppName}"; Flags: uninsdeletekey
Root: HKCR; Subkey: "SystemFileAssociations\.markdown\shell\LawPDF.ConvertToPdf"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#MyAppIconName}"
Root: HKCR; Subkey: "SystemFileAssociations\.markdown\shell\LawPDF.ConvertToPdf\command"; ValueType: string; ValueData: """{app}\{#MyAppExeName}"" --convert-to-pdf ""%1"""
Root: HKCR; Subkey: "SystemFileAssociations\.txt\shell\LawPDF.ConvertToPdf"; ValueType: string; ValueData: "Convert to PDF with {#MyAppName}"; Flags: uninsdeletekey
Root: HKCR; Subkey: "SystemFileAssociations\.txt\shell\LawPDF.ConvertToPdf"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#MyAppIconName}"
Root: HKCR; Subkey: "SystemFileAssociations\.txt\shell\LawPDF.ConvertToPdf\command"; ValueType: string; ValueData: """{app}\{#MyAppExeName}"" --convert-to-pdf ""%1"""
Root: HKCR; Subkey: "SystemFileAssociations\.text\shell\LawPDF.ConvertToPdf"; ValueType: string; ValueData: "Convert to PDF with {#MyAppName}"; Flags: uninsdeletekey
Root: HKCR; Subkey: "SystemFileAssociations\.text\shell\LawPDF.ConvertToPdf"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#MyAppIconName}"
Root: HKCR; Subkey: "SystemFileAssociations\.text\shell\LawPDF.ConvertToPdf\command"; ValueType: string; ValueData: """{app}\{#MyAppExeName}"" --convert-to-pdf ""%1"""
Root: HKCR; Subkey: "SystemFileAssociations\.log\shell\LawPDF.ConvertToPdf"; ValueType: string; ValueData: "Convert to PDF with {#MyAppName}"; Flags: uninsdeletekey
Root: HKCR; Subkey: "SystemFileAssociations\.log\shell\LawPDF.ConvertToPdf"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#MyAppIconName}"
Root: HKCR; Subkey: "SystemFileAssociations\.log\shell\LawPDF.ConvertToPdf\command"; ValueType: string; ValueData: """{app}\{#MyAppExeName}"" --convert-to-pdf ""%1"""
Root: HKCR; Subkey: "SystemFileAssociations\.csv\shell\LawPDF.ConvertToPdf"; ValueType: string; ValueData: "Convert to PDF with {#MyAppName}"; Flags: uninsdeletekey
Root: HKCR; Subkey: "SystemFileAssociations\.csv\shell\LawPDF.ConvertToPdf"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#MyAppIconName}"
Root: HKCR; Subkey: "SystemFileAssociations\.csv\shell\LawPDF.ConvertToPdf\command"; ValueType: string; ValueData: """{app}\{#MyAppExeName}"" --convert-to-pdf ""%1"""
Root: HKCR; Subkey: "SystemFileAssociations\.json\shell\LawPDF.ConvertToPdf"; ValueType: string; ValueData: "Convert to PDF with {#MyAppName}"; Flags: uninsdeletekey
Root: HKCR; Subkey: "SystemFileAssociations\.json\shell\LawPDF.ConvertToPdf"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#MyAppIconName}"
Root: HKCR; Subkey: "SystemFileAssociations\.json\shell\LawPDF.ConvertToPdf\command"; ValueType: string; ValueData: """{app}\{#MyAppExeName}"" --convert-to-pdf ""%1"""

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\{#MyAppIconName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\{#MyAppIconName}"; Tasks: desktopicon

[Run]
Filename: "ms-settings:defaultapps?registeredAppMachine=LawPDF"; Description: "Choose LawPDF as the default PDF reader"; Flags: shellexec postinstall skipifsilent unchecked nowait
Filename: "{app}\{#MyAppExeName}"; Description: "Launch {#MyAppName}"; Flags: nowait postinstall skipifsilent
