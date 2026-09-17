# Releasing LawPDF

A release is made from a curated source commit, with private research and training inputs excluded. Use the pinned Rust toolchain and committed lockfile. All build/dependency trees must live outside Box. Check the runtime asset manifest, third-party notices, and per-platform license inventories after dependency changes.

1. Complete implementation before the final verification phase. Run desktop normal/development suites, dependency audit, platform native/runtime checks, persistence fault cases, and the checks in `PRODUCTION_PLAN.md`. Document unverified accessibility, field-reliability, and platform-signing requirements explicitly.
2. Build both platform packages from the same commit. The Windows job installs the exact installer, compares its hash, checks product/file versions and every LawPDF Start Menu shortcut, and requires successful native/context/model runtime status. Retain the installation evidence with the release. A hosted runner is not evidence of testing on a user's own Windows hardware.
3. Tag the reviewed version. The release workflow produces artifacts, GitHub provenance attestations, and a **draft** release. It refuses to overwrite an existing release. Review the exact commit and platform results; never publish a draft merely because a build finished.
4. Download all three packages and `UPDATE-MANIFEST.json` to an external release directory. Verify the GitHub artifact attestations for repository `yonathanarbel/LawPDF` and the release workflow before signing. Recompute hashes and byte sizes; the manifest must refer to that same source commit and version. Example: `gh attestation verify <artifact> --repo yonathanarbel/LawPDF --signer-workflow yonathanarbel/LawPDF/.github/workflows/release.yml`.
5. Sign the exact verified manifest on the maintainer's Mac:

   ```sh
   swift -module-cache-path /private/tmp/lawpdf-signing-cache scripts/update-signing.swift sign \
     packaging/update-public-key.hex /private/tmp/lawpdf-release/UPDATE-MANIFEST.json \
     /private/tmp/lawpdf-release/UPDATE-MANIFEST.sig
   ```

   The initial key was created in macOS Keychain with the utility's `init` operation. Do not export it into a source file or shell argument. Keep an approved secure recovery plan for the signing identity; losing it interrupts automatic-update continuity.
6. Upload the detached signature, verify it against the committed public key, and exercise staging/restart/rollback with disposable app installations. The actual packages remain unchanged after their hashes are signed. Publish only after the gate evidence and accurate signing-status/install notes are attached.

The source signing policy is separate from platform credentials. Until SignPath approves Windows signing, report its absence. The Mac ZIP remains ad hoc signed and unnotarized under the free-distribution plan. Use Apple's documented user approval flow for installation; do not tell users to disable Gatekeeper globally. The Mac updater checks the declared minimum OS and bundled model runtime before replacement. It keeps `LawPDF.app.update-old` beside the installation until the next update; if a newly launched process subsequently fails, quit it and restore this retained bundle or the previous known-good GitHub package while keeping recovery data. LaunchServices accepting a launch is not proof of a healthy reader session. In-app updates reject downgrades.

Android has a separate release gate: lifecycle/process-death/provider-permission instrumentation, local annotation restoration, flattened-export semantics, and an Android signing identity. A desktop release does not certify Android. Do not publish an Android production build based solely on desktop checks.

For Android emulator verification, build `assembleDebug assembleDebugAndroidTest`, install both APKs on a disposable AVD, and run `scripts/verify-android-persistence.py --adb <SDK>/platform-tools/adb --serial emulator-<port> --avd-name <QA-AVD>`. The driver first runs four lifecycle/provider-copy checks, then commits a mark, force-stops the app, and restores it in a different process. It refuses physical devices and an unexpected AVD name.
