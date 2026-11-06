# OmniSync

<img src="assets/omnisync.svg" width="96" alt="OmniSync burnt-orange Möbius loop">

**Your files. Your Google Drive. You stay in control.**

OmniSync is a Windows desktop backup application written primarily in Rust. It backs up selected local folders and individual files to your Google Drive without moving them into a special sync folder. A Tauri interface, tray controls, native Explorer extension and Windows installer share the approved burnt-orange Möbius identity.

Repository: [william-pyle/OmniSync](https://github.com/william-pyle/OmniSync)

First time connecting Google Drive? Follow the [Google Cloud and sign-in walkthrough](#google-drive-setup) below.

Current development installer: [OmniSync Setup 0.1.1 x64](installer/releases/OmniSync-Setup-0.1.1-x64.exe) · [SHA-256 checksum](installer/releases/OmniSync-Setup-0.1.1-x64.exe.sha256). This build is unsigned; read the status and release checks below before use.

## Status and scope

This is the **0.1.1 development implementation** of the approved OmniSync design. It contains working application and installer source, generated icons, native Windows integration, automated tests and a Windows build workflow. It is not a claim of production certification. The local build is unsigned; Google account integration and machine-wide installation must be exercised in a disposable Windows test environment before relying on it for irreplaceable data.

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
| Cleanup | Review excluded files, select their known cloud versions and move them to Google Drive Trash; local files stay intact |
| Account | One Google account per Windows profile; system-browser desktop OAuth with PKCE and loopback callback |
| Installation | Windows x64 NSIS setup/uninstall; optional Explorer components; no automatic Explorer restart |
| Modern menu | Sparse identity package source and packaging script; activation requires the actual publisher's signing identity |

The app deliberately calls its operation **backup**. It does not implement two-way synchronization, cloud placeholders, a virtual drive or automatic propagation of local deletes into Google Drive. “Sync now” means run the backup queue immediately.

## Changes in 0.1.1

- Explorer reads a committed status snapshot during refresh. An independent heartbeat keeps it available during long uploads, and changed paths receive targeted Shell refresh notifications.
- Explorer badges use a small lower-left footprint. The app and animated tray keep the approved visual identity.
- Backed-up counts advance at recovery-catalog checkpoints during a batch. Already cataloged files stay counted when another file is pending.
- Include/exclude reviews open immediately with a calculating indicator. Apply becomes available when the impact is ready, and canceled results stay closed.
- **Rules > Clean up excluded backups** lets you select and trash known cloud copies of explicitly excluded files. See [cleanup instructions](#clean-up-excluded-cloud-backups).

### Upgrade from 0.1.0

1. Choose **Quit OmniSync** from its tray menu, then run the 0.1.1 installer. Keep the Explorer components enabled if you use them.
2. Launch OmniSync from the updated Start menu shortcut. Verify **0.1.1** in Settings. Existing folders, rules, account state and local backup records are retained; the sign-in startup entry follows your saved preference and is refreshed to the new executable.
3. When convenient, save your work and sign out of Windows and back in, or restart **Windows Explorer** yourself from Task Manager. Windows keeps an already loaded overlay DLL and its images in memory; F5 alone cannot load this update. Setup does not restart Explorer for you.
4. Allow the next backup to publish a recovery catalog. Older records need a one-time catalog coverage update before becoming green; unchanged file bytes are not uploaded again for this migration. Resume backup if it is paused.

The original 0.1.0 installer remains in `installer/releases/` as a historical artifact. Setup uses versioned application directories so an old DLL that Explorer still has open does not block the new payload. Older version directories can remain after an upgrade; use the current Start menu shortcut rather than an old executable.

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
out/OmniSync-Setup-0.1.1-x64.exe
out/OmniSync-Setup-0.1.1-x64.exe.sha256
out/payload/0.1.1/
```

`out/`, `target/` and `node_modules/` are ignored. Use a GitHub Actions artifact or a deliberately published GitHub release to distribute binaries; do not commit the build cache. The Windows workflow uploads an unsigned development installer on a successful build.

Reviewed installers and their checksums are versioned under `installer/releases/`. Local builds go to `out/`; they do not silently replace a checked-in artifact. The release script embeds the frontend using Tauri's `custom-protocol` feature and remaps local build paths. A release build without the production protocol feature is rejected at compile time.

`-SkipTests` is available for a packaging-only rebuild after tests have already passed. It is not the default.

## Google Drive setup

This development build requires you to create a Google OAuth **Desktop app** client once, download its JSON configuration, and import that file into OmniSync. A Google Cloud project holds the API settings and client identity; your backed-up files still go to your Google Drive. You do not need to write code or deploy a server for these steps.

The walkthrough uses Google's console terminology checked on November 5, 2026. Keep the **same project selected** throughout. A personal Gmail account normally uses the **External** audience; a managed work/school account may require its administrator's approval.

### 1. Create or select the Google Cloud project

1. Open [Google Cloud Console](https://console.cloud.google.com/) and sign in. For a personal setup, using the account whose Drive you will back up to keeps account selection straightforward.
2. Click the **project selector** near the top-left, beside the Google Cloud logo. It may say **Select a project** or show an existing project's name.
3. To reuse a project, select it. Otherwise choose **New project**, or open the [Create a project page](https://console.cloud.google.com/projectcreate) directly.
4. Enter **OmniSync Personal** as the project name. The generated project ID is fine. For a personal account, use **No organization** if that choice is shown; work accounts may require an organization/location.
5. Click **Create**, wait for completion, then use the project selector to select **OmniSync Personal**. Confirm that its name is displayed in the top bar before continuing.

Google's reference: [Create a Google Cloud project](https://developers.google.com/workspace/guides/create-project).

### 2. Enable Google Drive API

1. With your OmniSync project selected, open **☰ → APIs & Services → Library**. You can also open the [Google Drive API library page](https://console.cloud.google.com/apis/library/drive.googleapis.com) directly.
2. If using the Library search, enter **Google Drive API** and select that exact result. **Google Drive Activity API** is a different service.
3. Click **Enable** and wait. If the page already shows **Manage** or **API enabled**, it is already enabled for the selected project.
4. Confirm that **Google Drive API** appears under **APIs & Services → Enabled APIs & services**.

That completes the step to "create or select your Google Cloud project and enable Google Drive API." Continue below to create the sign-in configuration OmniSync needs. See Google's [API enablement instructions](https://developers.google.com/workspace/guides/enable-apis).

Google currently describes standard Drive API usage as available at no additional cost, with quota/billing changes planned for higher usage. Drive storage capacity is separate. Check the current [Drive API usage and pricing page](https://developers.google.com/workspace/drive/api/guides/limits#pricing) before enabling paid services or requesting higher quotas.

### 3. Set up the consent screen and your test account

1. Open **☰ → Google Auth platform → Branding**, or the [Branding page](https://console.cloud.google.com/auth/branding). If prompted, click **Get started**.
2. Set the app name to **OmniSync Personal**, choose your support email, and continue.
3. For personal Gmail, choose **External** as the audience. **Internal** is for an eligible Google Workspace organization.
4. Supply your contact email. Read the policy, accept it if you agree, and finish creating the configuration.
5. Open **Google Auth platform → Audience**. Keep **Testing** for the initial connection check.
6. Under **Test users**, click **Add users**, enter the exact Google email you will select when signing into OmniSync, and save.

Existing configurations can be edited in Branding and Audience. Older instructions may call this area **OAuth consent screen**. Google's [consent configuration guide](https://developers.google.com/workspace/guides/configure-oauth-consent) covers the current flow.

### 4. Add OmniSync's Drive permission

In **Google Auth platform → Data Access**, choose **Add or remove scopes**. Find/select the scope below, or enter it in the manual scope field if offered, then update and save:

```text
https://www.googleapis.com/auth/drive.file
```

This matches OmniSync's implemented permission request and grants access to app-created or explicitly authorized files. Broad `drive` access is unnecessary for this build. Google classifies `drive.file` as non-sensitive. See [Drive API scopes](https://developers.google.com/workspace/drive/api/guides/api-specific-auth).

### 5. Create and download the Desktop app client

1. Open **Google Auth platform → Clients**, or the [Clients page](https://console.cloud.google.com/auth/clients).
2. Choose **Create client** and set **Application type** to **Desktop app**.
3. Name it **OmniSync Windows**, then click **Create**. If an optional AI-agent checkbox appears, leave it unchecked; OmniSync is a desktop backup client.
4. Download the client JSON using **Download JSON** in the creation dialog or the download control associated with that client. Save it outside the Git repository, such as in your Downloads folder. Keep a private recovery copy of this configuration.

OmniSync imports the whole downloaded JSON file; you do not need to paste a client ID into its source. It expects an `installed` section. A **Web application**, API key or service-account JSON will not work with its importer. Desktop sign-in uses a local loopback callback; you do not configure web origins or a hosted redirect URL.

References: [Create a Desktop app client](https://developers.google.com/workspace/guides/create-credentials#desktop-app), [Google's desktop JSON setup example](https://developers.google.com/workspace/drive/api/quickstart/python#authorize_credentials_for_a_desktop_application), [Desktop OAuth and loopback redirects](https://developers.google.com/identity/protocols/oauth2/native-app).

### 6. Import the file and connect OmniSync

1. Open the installed OmniSync application and select **Settings**.
2. Click **Import desktop OAuth JSON** and select the file downloaded in step 5. A successful import enables **Connect**.
3. Click **Connect**. In your system browser, choose the account added under Test users and approve the requested Drive permission. Complete the flow within five minutes; otherwise click Connect again.
4. Return to OmniSync. Settings should show the account and the sidebar should say **Connected to Google Drive**. The browser callback alone is not proof that connection succeeded.
5. Add a small disposable test folder, review and apply its selection, then run **Sync now**. Resume first if paused. Check its uploaded files in Drive and test restoring one before selecting important folders.

The app creates its own visible backup folders in Drive. It does not require an existing cloud folder to be shared with a service account.

### Testing mode and common setup problems

For an External OAuth app left in **Testing**, Google's refresh token expires after **seven days** when requesting Drive access. Reconnect when that happens. For ongoing use, review **Audience → Publishing status** and Google's requirements for moving to **In production**; reconnect afterward to obtain a fresh authorization. Google's publishing status concerns OAuth access, not the production readiness of this OmniSync development build. See [refresh-token expiration](https://developers.google.com/identity/protocols/oauth2#expiration) and [Google Auth platform guidance](https://support.google.com/cloud/answer/15544987).

| Problem | What to check |
| --- | --- |
| Cannot create a project or enable the API | Confirm the signed-in account and selected project; a managed account may need its administrator to grant access |
| Google says the app is in testing or access is denied | Add the exact sign-in email under **Audience → Test users** in the project that owns the downloaded client |
| API is disabled / `accessNotConfigured` | Enable **Google Drive API** in the same project as that client |
| Import rejects the JSON | Download a **Desktop app** client JSON; do not use an API key, web client or service-account key |
| Sign-in reports `redirect_uri_mismatch` | Recheck that the imported client is **Desktop app**; do not add guessed redirect URLs to a web client |
| Access stops after about a week | Check the External/Testing token-expiration rule above and reconnect |
| Work/school account blocks consent | Ask the organization's administrator to review the app's requested access |

### Account access and storage

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

The engine watches selected roots, debounces notifications and reconciles content. It also performs a periodic full reconciliation. A watcher event immediately changes affected green files and their parent folders to pending while preserving unrelated entries. Unknown changes request a full reconciliation. Access-only events are ignored, and publication preserves changes that arrived after a scan started.

For each changed included file:

1. Verify the source location and root identity.
2. Open a Windows file handle that permits readers but denies concurrent writes/deletes during capture.
3. Stream to a private immutable staging file while computing SHA-256 and MD5; flush it to disk.
4. Save the job in SQLite before requesting an upload. Persist the pre-generated Drive target ID before creation, and the protected resumable URL before sending chunks.
5. Send chunks of up to 8 MiB. Save acknowledged offsets. On retry, query the saved session or reconcile the pre-generated target ID before creating a replacement session.
6. Verify Drive's size and SHA-256, falling back to MD5 when Drive does not supply SHA-256.
7. Commit the verified version and durable catalog-dirty flag together. Publish a recovery catalog, then reconcile current local bytes before displaying completion.

The queue publishes a catalog after eight completed files, after a completion when five seconds have elapsed, and before starting another large file when completed files are waiting. These are safe checkpoints, not a five-second completion guarantee during a slow cloud request. The Overview polls every 1.5 seconds and counts each current version only after its exact remote ID is covered by a published catalog. A later pending file does not demote already covered versions.

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

**No automatic remote deletion or version pruning is enabled in 0.1.1.** The approved design proposed keeping the current version plus ten prior versions; the configuration's `priorVersions` field reserves that policy. Excluding a file retains its cloud copies by default. The separate reviewed cleanup action below removes only the copies you select. Storage usage otherwise grows as files change.

Upload requests that completed just before a crash can leave a recoverable staged job or an unreferenced remote snapshot. The saved target ID prevents blind duplicate creation for that job. There is no automatic orphan-deletion pass in this release.

Protocol reference: [Google Drive resumable uploads and pre-generated IDs](https://developers.google.com/workspace/drive/api/guides/manage-uploads).

## Clean up excluded cloud backups

1. Save your exclusion rules, or disable a configured source, and review the resulting inclusion/exclusion preview.
2. Open **Rules > Clean up excluded backups**. The dialog appears immediately while OmniSync finds known cloud versions for explicitly excluded files.
3. Select individual files or **Select all**. Nothing is selected by default. Each row shows the number and total size of that file's **current and previous cloud copies**.
4. Choose **Move selected copies to Trash**. OmniSync yields any active backup at its next safe checkpoint, then checks the connected account, exact Drive IDs, ownership, OmniSync device/item tags and stored size/hash before each removal.
5. Read the result. Successful removals are recorded individually; failed or changed copies remain available for another review. Closing the progress dialog does not cancel an already confirmed cleanup.

**Local files are never deleted by this action.** Cleanup uses Drive's Trash, not permanent deletion or Empty Trash. Google normally deletes trashed files after 30 days, and they still consume storage until permanently deleted. You can inspect or restore them through Google Drive's Trash during that period. [Google Drive trash behavior](https://developers.google.com/workspace/drive/api/guides/delete).

The list is based on this Windows profile's known backup records. It includes explicit exclude rules and disabled configured roots, respecting exact include overrides. Removing a root from configuration, losing access to a source, or encountering a safety exclusion alone does not authorize cleanup. Recovery catalogs, folders, another device's files, modified cloud snapshots and unknown/orphan objects are not automatically removed.

The review expires after ten minutes and must be repeated if configuration or account identity changes. Configuration changes wait until confirmed cleanup finishes. A durable local journal and per-version updates allow a fresh review to retry remaining work after partial failures; already absent or trashed copies are reconciled without another removal.

The next active backup publishes an updated recovery catalog. If backup is paused or automatic backup is off, resume it or use **Sync now** after resuming. Older retained catalogs may still list versions you trashed; those references can no longer restore those copies through OmniSync. Restoring a trashed object in Google Drive does not automatically re-add it to the local version list. Re-including the local file creates a new verified backup if needed.

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

Explorer calls the overlay handler when it needs status, including a refresh. The handler immediately performs a bounded lookup in a per-user shared-memory table; it never scans a folder or calls Google. Two snapshot banks let it read the last committed table while a new one is being prepared. An independent heartbeat runs every 500 ms, including during uploads, with a 15-second expiry if the engine becomes unavailable. The table supports 262,144 entries. Invalid, stale or unavailable cache data returns no badge.

When states change, OmniSync coalesces targeted `SHCNE_UPDATEITEM` notifications so visible Explorer items can be queried again without waiting for F5. Notifications run separately from the heartbeat; long paths use Shell item IDs. The ICO files include transparent padding around small lower-left badges. Explorer still controls scheduling and the limited overlay pool; its cached DLL and images require a reload after an upgrade.

Right-click actions are **Back up with OmniSync…**, **Exclude from OmniSync…** and **Show OmniSync status**. Include/exclude opens the app's review flow immediately with a calculating indicator. Apply stays disabled until the result is ready, and Cancel discards late results. A cold application launch still takes startup time. The command does not mutate a configuration from inside Explorer. The initial commands support a single selected file or folder. The standard installer places them in **Show more options** on Windows 11.

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
& "$env:ProgramFiles\OmniSync\0.1.1\omnisync-setup.exe" restore `
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

Binaries live under `%ProgramFiles%\OmniSync\0.1.1`. Setup checks whether OmniSync is running and asks you to close it rather than terminating it. It does not auto-launch an elevated backup app after installation.

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
- Reviewed Trash cleanup with successful, already-trashed, unowned and changed copies; partial failure, restart and subsequent catalog publication. Automated cleanup tests use scripted responses and never alter a real account.
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
