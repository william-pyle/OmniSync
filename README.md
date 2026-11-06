# OmniSync

<img src="assets/omnisync.svg" width="96" alt="OmniSync burnt-orange Möbius loop">

**Your files. Your Google Drive. You stay in control.**

OmniSync is a Windows desktop backup application written primarily in Rust. It backs up selected local folders and individual files to your Google Drive without moving them into a special sync folder. A Tauri interface, tray controls, native Explorer extension and Windows installer share the approved burnt-orange Möbius identity.

Repository: [william-pyle/OmniSync](https://github.com/william-pyle/OmniSync)

Initial development installer: [OmniSync Setup 0.1.0 x64](installer/releases/OmniSync-Setup-0.1.0-x64.exe) · [SHA-256 checksum](installer/releases/OmniSync-Setup-0.1.0-x64.exe.sha256). This build is unsigned; read the status and release checks below before use.

## Status and scope

This is the **0.1.0 development implementation** of the approved OmniSync design. It contains working application and installer source, generated icons, native Windows integration, automated tests and a Windows build workflow. It is not a claim of production certification. The local build is unsigned; Google account integration and machine-wide installation must be exercised in a disposable Windows test environment before relying on it for irreplaceable data.

See [the verification record](docs/verification.json) for the checks actually performed and the remaining release gates. A compiled installer is not evidence that installation, Google OAuth or Explorer activation has been tested on every supported machine.

| Area | This implementation |
| --- | --- |
| Sources | Multiple fixed local NTFS directories or individual files; original files stay in place |
| Rules | Exact file, subtree, filename glob and path glob; include overrides; preview before saving; configuration undo |
| Backup | One-way local → Google Drive; immutable staged snapshots, resumable uploads, remote checksum verification |
| Controls | Pause for one hour, eight hours or indefinitely; resume; Sync now; automatic or manual backup |
| Tray | App continues when its window closes; open, sync, pause/resume and quit; animated syncing icon |
| Explorer | Five COM overlay handlers and three `IExplorerCommand` actions; legacy menu registration included |
| Slot conflicts | Explicit selection, registration fingerprints, protected recovery receipts, rollback and guarded restoration |
| Restore | Current and historical file versions, including recovery catalogs from Drive; verify bytes; keep existing destination files |
| Account | One Google account per Windows profile; system-browser desktop OAuth with PKCE and loopback callback |
| Installation | Windows x64 NSIS setup/uninstall; optional Explorer components; no automatic Explorer restart |
| Modern menu | Sparse identity package source and packaging script; activation requires the actual publisher's signing identity |

The app deliberately calls its operation **backup**. It does not implement two-way synchronization, cloud placeholders, a virtual drive or automatic propagation of local deletes into Google Drive. “Sync now” means run the backup queue immediately.

## Requirements

For use:

- Windows 11 x64, a fixed NTFS volume and Microsoft Edge WebView2 Runtime.
- A Google account with enough available Drive storage.
- A Google Cloud **Desktop app** OAuth client with Drive API enabled. Development builds import this JSON locally; no publisher-owned OAuth client is embedded.
- Administrator approval for machine-wide installation and Explorer registration changes. Normal backup runs as the signed-in user.

For development:

- Rust **1.98.1**, Cargo, the MSVC x64 toolchain and the Windows SDK.
- Visual Studio 2022 Build Tools or Visual Studio with **Desktop development with C++**.
- Node.js **24** and npm. Node 22.12 or later is required by the frontend toolchain; the recorded build uses Node 24.
- NSIS 3 to build the installer. Set `NSIS_DIR` if it is not in one of the paths detected by `scripts/build.ps1`.
- Optional Windows SDK `MakeAppx.exe` and `SignTool.exe` for signed release packaging.

`Cargo.lock` and `package-lock.json` are committed. The workspace is self-contained; it does not depend on the design workspace or its installed packages.

## Quick start from source

```powershell
git clone https://github.com/william-pyle/OmniSync.git
cd OmniSync
npm ci
npm run desktop
```

`npm run desktop` starts the local Vite frontend and the Rust desktop host. Closing the window keeps OmniSync running in the tray. Use **Quit OmniSync** in its tray menu to exit.

Opening the Vite URL in an ordinary browser does not simulate a backup engine. It displays instructions to open the desktop app. The production UI only calls a fixed Rust command dispatcher; it has no arbitrary shell execution API.

Useful commands:

```powershell
npm run build                  # Type-check and bundle the interface
cargo check --workspace --locked
cargo test --workspace --locked
cargo fmt --all -- --check
npm run audit:source           # Check the distributable source set
npm run test:ui                # Edge interface tests with explicit mock IPC
npm run icons                  # Rebuild approved vector-derived icons
```

### Build the Windows installer

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build.ps1
```

The script installs locked npm dependencies, builds the interface, runs Rust tests, builds release binaries, stages only the required files, smoke-tests the actual Explorer DLL and invokes NSIS. The DLL check instantiates all five overlay classes and three context commands, checks icon files and unmanaged-path behavior, and verifies COM lifetime without registering or invoking a command. It does **not** install the result, sign in to Google, change Explorer registrations or restart an application.

Outputs:

```text
out/OmniSync-Setup-0.1.0-x64.exe
out/OmniSync-Setup-0.1.0-x64.exe.sha256
out/payload/0.1.0/
```

`out/`, `target/` and `node_modules/` are ignored. Use a GitHub Actions artifact or a deliberately published GitHub release to distribute binaries; do not commit the build cache. The Windows workflow uploads an unsigned development installer on a successful build.

The initial check-in intentionally includes one reviewable installer and its checksum under `installer/releases/`. Later local builds go to `out/`; they do not silently replace that checked-in artifact. The release script embeds the frontend using Tauri's `custom-protocol` feature and remaps local build paths. A release build without the production protocol feature is rejected at compile time.

`-SkipTests` is available for a packaging-only rebuild after tests have already passed. It is not the default.

## Google Drive setup

1. Create or select your Google Cloud project and enable **Google Drive API**.
2. Configure the OAuth consent screen. For a development project in testing mode, add your Google account as a test user. Follow Google's current publishing requirements for distribution to other people.
3. Create an OAuth client of type **Desktop app**. Download its JSON outside this repository.
4. In OmniSync, open **Settings → Import desktop OAuth JSON**. The importer expects the Google JSON's `installed` section. It rejects a web-client JSON.
5. Choose **Connect**. Complete consent in the system browser. The redirect is an ephemeral `127.0.0.1` loopback listener with a random state value and PKCE verifier. The listener times out after five minutes.
6. Add a folder, review the inclusion counts and apply. The app will create its own visible backup folders in your Drive.

The requested scope is `https://www.googleapis.com/auth/drive.file`. This limits access to files created or authorized for this OAuth application. OmniSync does not adopt Google Drive for desktop's existing **Computers** backup tree. Changing OAuth client projects can also change which existing recovery catalogs are visible.

Google Desktop OAuth client secrets are public-client configuration, not a secure substitute for PKCE. Nevertheless, OmniSync stores the imported configuration and refresh/access tokens under Windows DPAPI, in a directory restricted to the current Windows user and SYSTEM. Tokens and resumable session URLs are never sent to the UI or printed in normal status messages.

This profile binds to Google's stable account permission ID. Reconnecting a different account is refused before queued jobs can be used against that account. Use a separate Windows user for a separate OmniSync profile in this release.

Disconnect pauses backup, attempts to revoke the refresh token and removes local authorization. It preserves the imported OAuth client configuration, backup catalog and remote files. A network failure can prevent remote revocation; revoke access in your Google account settings when needed.

Official setup references: [Desktop OAuth](https://developers.google.com/identity/protocols/oauth2/native-app), [Drive API scopes](https://developers.google.com/workspace/drive/api/guides/api-specific-auth).

## Interface and everyday flow

The approved interface uses a dark navy surface, warm burnt-orange accents, Segoe UI typography, restrained borders and the Möbius mark. Light, dark and system themes use the same state vocabulary.

| Screen | Purpose |
| --- | --- |
| Overview | Real backup status, counts, pause/resume, Sync now, onboarding and selected folders |
| Folders | Add directories or individual files; disable or remove sources after an impact preview |
| Rules | Inspect, add and remove exclusions and includes; undo the previous saved configuration |
| Activity | Search records, filter by state, inspect the winning rule and see verified versions |
| Restore | Restore versions from this profile or load a cloud recovery catalog |
| Explorer | Inspect actual overlay registrations; register OmniSync; review takeover; restore receipts |
| Settings | OAuth, account, automatic backup, metered networks, startup, appearance, staging and bandwidth |

Changes that affect selection are previewed and tied to a specific configuration revision. A preview expires after ten minutes. If another operation changes the configuration, the old preview cannot silently overwrite it.

The window's close button hides it in the tray. **Quit** stops the process. Backup does not run as SYSTEM and does not run after sign-out. Enable **Start at Windows sign-in** if desired; it is off by default.

### Pause and motion

Pause is an actual engine state. A currently transmitted chunk can finish before the next checkpoint; subsequent chunks and new jobs stop. The staged snapshot and resume session stay durable. Sync now is disabled while paused and the backend enforces the same rule.

The app mark turns once every six seconds, with a three-second highlight flow, while actively uploading. The tray uses 24 pre-rendered frames at a maximum of four updates per second. Paused, idle and disconnected states are static. Reduced-motion preferences suppress rotation while retaining state text and numerical progress. Explorer overlay images are static: they are status marks, not an animation surface.

## Source eligibility and rules

Release 0.1 targets ordinary files on fixed local NTFS disks. Source roots must be absolute, non-overlapping, outside OmniSync's own state directory and associated with the volume/file identity recorded when selected.

Non-overridable checks reject symlinks, junctions, reparse points, offline/cloud placeholders, EFS files, network/removable roots and case-sensitive directories. OmniSync does not hydrate another provider's placeholders. A file open for writing may wait with an issue until it can be safely captured. Files cannot opt out of these checks with an include rule.

Within eligible sources, precedence is:

1. Exact file override.
2. Most-specific matching subtree rule.
3. Folder-scoped filename/path patterns.
4. Global filename/path patterns.
5. Included by the selected folder's default.

At equal specificity, a folder-scoped rule outranks a global rule, and later ordered rules win. Patterns are case-insensitive. `/` separates path segments; `**` can span zero or more directory segments. Rule paths cannot contain drive letters, absolute roots, empty segments, `.` or `..` traversal.

| Type | Example | Meaning |
| --- | --- | --- |
| Filename pattern | `*.tmp` | Match a filename at any depth |
| Path pattern | `**/cache/**` | Match files beneath directories named cache |
| Subdirectory | `Projects/demo/build` | Match that subtree |
| Exact file | `Projects/demo/build/keep.txt` | Match only that file; an include overrides the excluded subtree |

Rules are not silently pre-populated with exclusions. Use the preview and **Activity** reasons to understand what will be included. Removing or disabling a source stops future uploads but preserves existing backups.

## Backup engine and durability

The engine watches selected roots, debounces notifications and reconciles content. It also performs a periodic full reconciliation. A watcher event immediately invalidates the shared Explorer cache; Explorer will not keep showing a supposedly fresh green badge while the engine is checking changes.

For each changed included file:

1. Verify the source location and root identity.
2. Open a Windows file handle that permits readers but denies concurrent writes/deletes during capture.
3. Stream to a private immutable staging file while computing SHA-256 and MD5; flush it to disk.
4. Save the job in SQLite before requesting an upload. Persist the pre-generated Drive target ID before creation, and the protected resumable URL before sending chunks.
5. Send chunks of up to 8 MiB. Save acknowledged offsets. On retry, query the saved session or reconcile the pre-generated target ID before creating a replacement session.
6. Verify Drive's size and SHA-256, falling back to MD5 when Drive does not supply SHA-256.
7. Commit the verified version and durable catalog-dirty flag together. Publish a recovery catalog, then reconcile current local bytes before displaying completion.

SQLite uses WAL mode and full synchronous writes. Staged files have a default 50 GiB budget and 5 GiB free-space reserve. A file larger than the available staging budget waits with an actionable issue; it is not streamed from a changing source. Failed upload jobs use capped exponential backoff. The upload-rate limit is enforced between chunks, so brief within-chunk bursts are expected.

Network cost is queried from Windows. With the default metered setting, metered, roaming, over-limit or unknown-cost connections wait. Turn that preference off if your otherwise usable network reports no cost information.

Current remote copies are checked periodically and before a new upload. Missing or changed cloud content becomes a conflict. Acknowledging a conflict creates a **new separate snapshot**, preserving the old remote object. This is not automatic two-way conflict merging.

### Remote layout and retention

This first implementation uses **append-only file snapshots**. It does not overwrite the bytes of a previously uploaded Drive object. This is a deliberate implementation refinement from the design's stable-current-file/copy-to-History proposal: it avoids depending on an unverified conditional-overwrite contract.

```text
OmniSync/
  Device <short device ID>/
    <selected root label>/
      Files/
        <relative directories>/
          report [20261106T020000Z-<snapshot ID>].pdf
    OmniSync-catalog-<snapshot ID>.json
```

The app's catalog presents the logical current file and previous versions. The timestamped original file bytes remain directly downloadable in Drive even without OmniSync. Each device currently creates its own top-level `OmniSync` folder; devices do not silently adopt folders that happen to share a name.

Recovery catalogs contain relative paths, root labels, version IDs and checksums. They omit local absolute root paths, OAuth tokens and account credentials. File names and root labels are still personal information; Drive stores them with the backup.

**No automated remote deletion or version pruning is enabled in 0.1.0.** The approved design proposed keeping the current version plus ten prior versions. This build conservatively retains all versions and catalogs; the configuration's `priorVersions` field reserves that policy but does not authorize deletion. Storage usage grows as files change. Perform deliberate cleanup in Google Drive only after verifying another usable copy. Deleting referenced versions can make those versions unavailable to restore.

Upload requests that completed just before a crash can leave a recoverable staged job or an unreferenced remote snapshot. The saved target ID prevents blind duplicate creation for that job. There is no automatic orphan-deletion pass in this release.

Protocol reference: [Google Drive resumable uploads and pre-generated IDs](https://developers.google.com/workspace/drive/api/guides/manage-uploads).

## Restore and disaster recovery

Select a file in **Activity** or **Restore** and choose a verified version. Pick an existing destination folder outside all configured backup roots. OmniSync downloads into a temporary file, verifies its size and both stored hashes, then publishes the restored file. Existing destination files are kept using a new restored name. Original ACLs, alternate data streams, EFS metadata and NTFS sparse allocation are not reconstructed; this is a backup of ordinary file content and logical names.

For a new Windows profile:

1. Install or build OmniSync.
2. Import the same Google OAuth application configuration and connect the original account.
3. Open **Restore → Find cloud recovery catalogs**.
4. Select the desired device/catalog and inspect its file versions.
5. Restore to a new local folder. Select that folder for future backup only after reviewing the restored content.

Catalog paths are treated as untrusted input. Absolute paths, traversal, Windows device names and invalid Windows filename characters are rejected. OmniSync does not restore directly into its application state or an active backup source.

## Explorer integration and overlay slots

The native DLL implements five `IShellIconOverlayIdentifier` COM classes:

| Badge | Meaning |
| --- | --- |
| Backed up | The current local content matches a verified cloud version and its recovery catalog has been published |
| Syncing | An included file is being uploaded |
| Pending | An included change is waiting, retrying or awaiting catalog publication |
| Excluded | A rule, disabled folder or non-overridable eligibility check excludes the item |
| Issue | An unreadable source, unavailable root, cloud conflict or other actionable problem |

Paused, disconnected, metered and authentication conditions also have app/tray text. They do not consume additional Explorer overlay slots. Folder badges summarize known child records. Files outside managed roots have no OmniSync badge.

Explorer never scans a folder or calls Google to determine a badge. It performs a bounded lookup in a per-user shared-memory table of hashed paths and states. The table uses a generation check, has a bounded capacity of 262,144 entries and expires after 15 seconds without a fresh publication. An unavailable, stale or malformed cache returns no badge. The app remains the authoritative detailed status view.

Right-click actions are **Back up with OmniSync…**, **Exclude from OmniSync…** and **Show OmniSync status**. Include/exclude opens the app's review flow; it does not mutate a configuration from inside Explorer. The initial commands support a single selected file or folder. The standard installer places them in **Show more options** on Windows 11.

### Taking slots from another provider

Windows has a shared, limited icon overlay pool. Registering OmniSync does not guarantee Windows will load all its handlers. The installer and **Explorer** screen offer optional takeover of specifically selected competing registrations.

- Inventory includes the registration name, CLSID, resolved DLL and a fingerprint of the registration tree.
- No competing registration is selected by default.
- OmniSync's own, unresolved and Windows-directory/system providers cannot be selected.
- Before removal, the helper checks the current fingerprint and writes a durable receipt under `%ProgramData%\OmniSync\OverlayBackups` with values, types, nested keys and security descriptors.
- The helper removes only the selected `ShellIconOverlayIdentifiers` subkeys. It does not unregister the provider's COM server, stop its process, alter its settings or remove its cloud data.
- On failure, it attempts restoration from the receipt. Receipts survive failed setup and uninstall.
- Restoration creates only absent keys. Existing/newer vendor registrations are preserved and reported, not overwritten.
- Explorer refresh is deferred. Sign out or restart Explorer yourself when convenient; OmniSync never does it automatically.

The setup recovery command is:

```powershell
# Run in an elevated terminal; substitute the receipt actually saved on your PC.
& "$env:ProgramFiles\OmniSync\0.1.0\omnisync-setup.exe" restore `
  "$env:ProgramData\OmniSync\OverlayBackups\<receipt>.json"
```

The receipt includes the original security descriptor; restoration reapplies its DACL. Windows controls the owner of the newly created key. Vendor repair/update processes can re-register handlers later, so takeover is recoverable configuration management, not a permanent reservation of Windows capacity.

Reference: [Microsoft overlay handler contract](https://learn.microsoft.com/en-us/windows/win32/shell/how-to-implement-icon-overlay-handlers).

## Installer flow

The NSIS source implements this sequence:

1. **Welcome** — product identity and development-build disclosure.
2. **Windows integration** — choose overlays and right-click commands.
3. **Review overlay slots** — inspect exact providers; optionally enter the numbered registrations to disable. The empty default keeps all providers.
4. **Ready to install** — review location, components, takeover selection and deferred Explorer refresh.
5. **Install** — copy the versioned payload, apply reviewed integration and register Start menu/Apps & features entries.
6. **Finish** — launch instructions, account/folder onboarding and recovery-receipt location.

Binaries live under `%ProgramFiles%\OmniSync\0.1.0`. Setup checks whether OmniSync is running and asks you to close it rather than terminating it. It does not auto-launch an elevated backup app after installation.

Uninstall removes owned integration and application files. Loaded shell DLLs may be scheduled for deletion at the next restart. The uninstaller does not force a restart and retains source files, cloud backups, per-user state and overlay recovery receipts. Disconnect and disable start-at-sign-in before uninstalling if you want authorization removed and no leftover user startup entry.

### Signing and Windows 11 modern menu

The normal development installer is unsigned. For a release, set the variable names listed in `.env.example` in your build environment and run:

```powershell
./scripts/build.ps1 -Signed
```

The script signs application binaries and the final installer with a certificate already available to SignTool. It does not create a publisher identity, import a certificate or store private keys in the repository.

The modern Windows 11 menu requires package identity. The source is in `installer/sparse/AppxManifest.xml.in`, with the same command CLSIDs as the native DLL. Build a sparse package with your real certificate subject:

```powershell
./scripts/build-identity.ps1 `
  -Publisher 'CN=<actual certificate subject>' `
  -PublisherDisplayName '<actual publisher name>' `
  -MakeAppx '<Windows SDK path>\makeappx.exe' `
  -Signed
```

Publishers must embed matching identity metadata into the desktop executable manifest, sign the package with a certificate trusted on the target PC, register it for the intended Windows users with the installed payload as its external location, and test activation and uninstall. The unsigned default installer does not pretend to provide this identity or install a test certificate.

Microsoft references: [Modern Explorer commands](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/integrate-packaged-app-with-file-explorer), [Sparse package identity](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/grant-identity-to-nonpackaged-apps).

## Repository map

```text
assets/                      Approved Möbius SVG, palettes, UI icons, ICOs, tray frames
src/                         TypeScript interface and styles
src-tauri/                   Rust desktop host, command boundary and tray lifecycle
crates/omnisync-core/         Rules, staging, SQLite, OAuth, Drive protocol and engine
crates/omnisync-windows/      DPAPI, path checks, shared cache and registration management
crates/omnisync-shell/        In-process Explorer COM DLL; no network or backup engine
crates/omnisync-setup/        Narrow setup/recovery command-line helper
installer/                   NSIS setup/uninstall and sparse identity manifest
scripts/                     Reproducible icons, build, packaging and source audit
tests/                       Approved rule fixtures and UI tests
docs/verification.json       Actual checks and remaining release gates
.github/workflows/           Windows build, tests and installer artifact
```

The first build hosts the UI and asynchronous Rust backup worker in one per-user process. Hiding the window leaves that worker running. This is simpler than the design's separate agent/UI executable proposal; a UI-process crash currently also stops backup. Persistent jobs support recovery on the next launch. Explorer stays in a separate native DLL, and elevated registry work stays in a separate setup helper.

## Local data and privacy

The installed application stores its profile under the Tauri application-local-data directory, normally `%LOCALAPPDATA%\app.omnisync.desktop`:

| File or directory | Contents |
| --- | --- |
| `state.sqlite`, `-wal`, `-shm` | Configuration revisions, known files, queue, account display information and Drive IDs |
| `credentials.dpapi` | Encrypted imported OAuth configuration and authorization tokens |
| `staging/` | Immutable upload snapshots and recovery-catalog staging files |
| `overlay-selection.json` | The specific names/fingerprints submitted for a takeover review |

This data is private runtime state. Keep it out of Git. The source ignores `.env` files, databases, logs, keys, credentials, build products and dependency caches. `.env.example` contains variable names with empty values only.

The app does not upload telemetry, diagnostics, local absolute paths or credentials to a separate OmniSync service. Backup data goes to the connected Google Drive. File contents are not encrypted before upload; Google Drive account security and storage controls apply. DPAPI protects local credentials at rest, not a compromised logged-in user session.

## Validation and release checklist

Automated checks cover rule precedence and the approved rule fixtures; traversal/containment and Windows names; remote verification predicates and upload acknowledgement bounds; credential-destination restrictions; SQLite revision/restart behavior; DPAPI round trips; staging immutability, integrity and active-writer handling; cache expiration/bounds; and registration value/digest handling. Frontend and native components must compile before packaging.

Before a public production release, use a dedicated Google test account and a clean Windows 11 VM to verify:

- OAuth consent, refresh, revocation, account mismatch and testing-mode expiry.
- Empty, small, Unicode, locked and multi-chunk files; offline/429/5xx recovery; crash after upload but before local/catalog commit; expired resumable sessions; quota exhaustion.
- Remote edits/removal, rename/move, disconnected roots and rule changes while an upload is in flight.
- Recovery on a fresh Windows profile, malicious catalog paths, existing restore files and hash mismatch.
- Administrator/standard-user install, machine-wide ownership, overlay capacity pressure, selected takeover, failure rollback, guarded undo, upgrade and uninstall with a loaded shell DLL.
- Both Explorer menu paths, package identity/signature, Start menu, sign-in startup, tray lifetime, high DPI, dark/light themes and reduced motion.
- Publisher signing, SmartScreen reputation, dependency/security review and Google consent branding.

Large-tree scaling, restore hardening under malicious concurrent filesystem mutation, signed sparse-package lifecycle, unattended upgrades and automatic retention cleanup need additional release engineering. The design's Drive changes-feed optimization, separate agent process, scheduled backup windows, automatic updater and encrypted backup mode are not enabled in this initial build.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| App does not start | Install/repair Microsoft Edge WebView2 Runtime; confirm Windows 11 x64 |
| OAuth import rejected | Use the downloaded **Desktop app** JSON with an `installed` section |
| Sign-in denied | Enable Drive API, configure consent and add the account as a test user |
| Backup waits on a usable network | Inspect the metered-network setting; unknown cost waits by default |
| File stays pending or has an issue | Open its Activity details; check writer locks, rule, staging space, account, network and conflict reason |
| Root identity changed | Re-select the intended folder; do not silently adopt a replacement volume |
| No Explorer badge | Check the tray is running, refresh integration inventory, inspect shared overlay capacity and allow a deferred Explorer refresh |
| Right-click action absent from the short menu | The unsigned installer uses **Show more options**; modern activation requires the signed identity package |
| A provider reclaimed its slots | Its update/repair may have re-registered handlers; refresh inventory and explicitly review again |
| Restore catalog is not listed | Connect the original account and OAuth application; confirm the catalog has not been removed |

## Initial check-in and contributions

This folder is intended to be the repository root. Review `git status` before committing; do not add account JSON, local state, private keys, installer build caches or unrelated workspace content. CI creates distributable artifacts from the committed source.

No open-source license has been selected by the owner in this initial delivery. The presence of source on GitHub does not grant a separate license to the OmniSync code or brand. Third-party dependencies retain their own licenses; publishers should produce and review dependency notices before distribution.
