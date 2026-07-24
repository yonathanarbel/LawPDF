# LawPDF workspace guidance

## Keep generated build output out of Box

This repository is inside a Box-synced folder. Never create Cargo build output or other generated dependency trees inside this repository; Box reports hundreds of upload errors for files under local `target*` directories.

- Set `CARGO_TARGET_DIR=C:\tmp\lawpdf-target` for every `cargo check`, `cargo test`, `cargo build`, and packaging command.
- Readable portions of the historical build trees removed on 2026-07-15 live under `C:\tmp\lawpdf-build-archive\`; Box-held files that returned device or access errors were discarded because they are reproducible Cargo cache.
- Do not recreate `target`, `target-*`, or similarly named build-output directories anywhere under this checkout.
- Git ignore rules do not prevent Box from attempting to sync physical files.

Example PowerShell setup:

```powershell
$env:CARGO_TARGET_DIR = 'C:\tmp\lawpdf-target'
cargo check --locked
```

For Android, use `android\build.ps1`. It redirects the Gradle user home,
project cache, and all module build directories to `C:\tmp\lawpdf-android`.
Never invoke bare Gradle in this checkout because its default project cache is
`android\.gradle`, which Box will try to sync.

## End development cycles with a Windows test install

At the end of every LawPDF development cycle:

- Build or download the newly completed Windows installer.
- Verify the installer checksum when it came from a release.
- Install that exact version on this Windows machine for user testing.
- Confirm that both the installed executable's product version and file version
  match the completed cycle.
- Run the installed executable with
  `--lm2-runtime-status --require-native --require-context` and require exit code
  `0` with `"requirements_met": true`.
- Ensure the user's Start Menu shortcut targets the newly installed executable,
  not an older per-user installation.
