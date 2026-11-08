# OmniSync

One-way Windows backups to a folder you choose: a local disk, Google Drive for desktop, another synced folder, a mapped drive or a network share. Source files stay where they are. Precise exclusions, retained versions, verified restores, tray controls and Explorer integration are included.

**[Download the latest Windows x64 release](https://github.com/william-pyle/OmniSync/releases/latest)**

Version 0.2 replaces the direct Google Drive API with ordinary filesystem copies. No Google Cloud project, OAuth client or provider credentials are required.

## Upgrade without losing your selections

1. Quit the old OmniSync from its tray menu and run the new installer. Do not uninstall or delete app data first.
2. Open OmniSync from the Start menu. Existing sources, exclusion rules, device identity and preferences remain saved.
3. Choose **Overview → Choose destination** (or Settings). Browse to a folder or enter an absolute path. Local, mapped and UNC paths are accepted.
4. Review and apply. All included source files are copied to this destination. If previously paused, select Resume.

Existing Google backups remain untouched; their local records are archived during migration. The new app copies from the selected source files, not from old Google copies. Switching destinations keeps your selections and old backups, then starts a fresh verified backup at the selected location. Nothing moves or deletes the original source files.

The old 0.1.x app has no updater. Install 0.2.0 manually once; later versions are offered by the startup update check. Do not run an old binary against the migrated database.

## How backups work

- `OmniSync/<device-id>/files/<source-name>-<source-id>/` contains readable files with their original relative paths.
- `versions/` holds immutable, SHA-256-verified versions. `catalogs/` holds recovery catalogs. `conflicts/` retains destination edits after acknowledgment.
- Each current readable file and its latest version snapshot are separate copies. Plan for at least twice the initial included data, plus historical versions, temporary copies and the configured free-space reserve. No automatic version pruning is performed.
- A temporary file is written, flushed and read back for integrity before publication. Interrupted transfers retain existing files and retry safely.
- Excluding a file, removing a source, deleting a local source or uninstalling retains existing backups. Source files are never silently restored or deleted.
- Restore verifies bytes, writes outside protected folders and uses a numbered filename if a destination already exists. Choose an old backup destination and use **Restore → Find recovery catalogs** to browse its versions.

The selected destination must exist, be writable and be separate from all sources and app data. A marker identifies the intended drive; an unavailable or substituted drive stops copying. Google Drive or another cloud client's own UI reports cloud-upload status. OmniSync's **Backed up** status confirms the local destination copy only.

Source support remains fixed local NTFS files and folders. Source links, junctions, offline placeholders and encrypted EFS files are excluded. Destination support now includes local, cloud-mounted, removable, mapped and network folders that provide normal Windows read/write/rename operations. Source rules and exclusion precedence are unchanged.

## Updates

The app checks the repository's latest stable GitHub Release on startup; Settings has a manual check. Checks are nonblocking and network failure does not stop folder backups. Download is optional. The app verifies the release asset's size and SHA-256 before offering **Close OmniSync & install**. Installing requires that explicit choice and Windows administrator confirmation. Setup waits for OmniSync to finish a checkpoint and close; it never terminates the app or Explorer. Open the new version from the Start menu afterward.

Current installers are **unsigned**. Windows may report an unknown publisher. A verified SHA-256 checksum does not substitute for an Authenticode publisher signature.

## Windows integration and data

Windows 11 x64 and Microsoft Edge WebView2 are required. The installer places versioned binaries in Program Files, adds a Start menu shortcut and offers five Explorer overlays and right-click commands. Overlay takeover is optional and reviews specific registrations before changing them. Recovery receipts remain in ProgramData/OmniSync/OverlayBackups. Windows' shared overlay limit still applies; a later sign-out may be needed to load a new shell DLL.

Per-user configuration, local records and the durable queue remain at `%LOCALAPPDATA%\app.omnisync.desktop`. SQLite migration archives legacy Google records and preserves configuration revisions. Old credential files are retained locally but are not loaded or used by 0.2. Uninstall retains sources, destination files and app data. Never publish this runtime directory or credentials.

The [HTML guide](docs/guide.html) has the full setup, destination layout, migration, restore and release instructions. Its editable source is [guide.json](docs/guide.json). Build it with `node scripts/build-docs.mjs`.

## Build and release

Prerequisites: Windows 11 x64, Rust 1.98.1 and Visual Studio MSVC C++ build tools/Windows SDK, Node 24, WebView2 and NSIS 3.11 (set `NSIS_DIR` if necessary).

```powershell
npm ci
npm run build
cargo test --workspace --locked
npm run test:ui
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build.ps1
```

The build produces `out/OmniSync-Setup-<version>-x64.exe` and `.exe.sha256`. It embeds the frontend, uses static CRT, smoke-tests the compiled Explorer DLL without registration and packages the installer. Local development: `npm run desktop`.

GitHub Actions compiles/tests pushes to `main` and pull requests. Push a matching version tag (`v0.2.0`, for example) to build and publish a Release after Rust tests, frontend build, source audit and Edge interface tests pass. Published assets come from the GitHub runner. Updating a release requires a new version; workflow reruns do not overwrite published installers.

Before tagging, update all version files (`Cargo.toml`, `Cargo.lock`, package manifests, `src-tauri/tauri.conf.json` and the NSIS fallback), add `docs/releases/<version>.txt`, then run `node scripts/check-version.mjs`. The release job uses the automatic repository token with `contents: write`; no personal token or Google secrets are required.

For publisher signing, the existing `-Signed` build switch uses `OMNISYNC_SIGNTOOL`, `OMNISYNC_CERT_THUMBPRINT` and `OMNISYNC_TIMESTAMP_URL`. Never commit signing keys. Modern Explorer sparse-package tooling remains in `scripts/build-identity.ps1` for a separately configured publisher identity.

## Verification and limits

Rust tests cover migration, destination changes, exclusions, immutable versions, restore collisions, interrupted copies, bad hashes, wrong destination identity, catalog progress, updater version/URL checks and Windows status cache behavior. Edge tests use explicit IPC fixtures to test the interface; they do not simulate real backup success in the shipped app. See [verification.json](docs/verification.json) for actual checked scope.

Real network-share/provider behavior, UAC installation/upgrade on a clean Windows VM and provider-specific online-only hydration should be exercised on the intended storage. No unattended cleanup or remote deletion is performed. This is file backup, not VSS or a transactional database/system-image backup.
