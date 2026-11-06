//! Read-only smoke test of the actual built COM DLL. No registrations are changed.
use anyhow::{Context, Result};
use omnisync_windows::integration::{CLSIDS, COMMAND_CLSIDS};
use std::ffi::c_void;
use windows::{
    core::{s, Interface, GUID, PCWSTR},
    Win32::{
        Foundation::{FreeLibrary, S_FALSE, S_OK},
        System::{
            Com::{CoTaskMemFree, IClassFactory},
            LibraryLoader::*,
        },
        UI::Shell::{IExplorerCommand, IShellIconOverlayIdentifier},
    },
};
pub fn shell() -> Result<()> {
    let directory = std::env::current_exe()?
        .parent()
        .context("Helper directory missing")?
        .to_path_buf();
    let path = omnisync_windows::wide(directory.join("omnisync_shell.dll"));
    unsafe {
        let module = LoadLibraryExW(
            PCWSTR(path.as_ptr()),
            None,
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
        )?;
        let result = (|| -> Result<()> {
            type GetClass = unsafe extern "system" fn(
                *const GUID,
                *const GUID,
                *mut *mut c_void,
            ) -> windows::core::HRESULT;
            type CanUnload = unsafe extern "system" fn() -> windows::core::HRESULT;
            let get: GetClass = std::mem::transmute(
                GetProcAddress(module, s!("DllGetClassObject"))
                    .context("Missing class factory export")?,
            );
            let unload: CanUnload = std::mem::transmute(
                GetProcAddress(module, s!("DllCanUnloadNow")).context("Missing unload export")?,
            );
            for (index, value) in CLSIDS.iter().chain(COMMAND_CLSIDS.iter()).enumerate() {
                let id = GUID::from_u128(u128::from_str_radix(
                    &value.trim_matches(['{', '}']).replace('-', ""),
                    16,
                )?);
                let mut raw = std::ptr::null_mut();
                get(&id, &IClassFactory::IID, &mut raw).ok()?;
                let factory = IClassFactory::from_raw(raw);
                if index < 5 {
                    let object: IShellIconOverlayIdentifier = factory.CreateInstance(None)?;
                    let mut icon = [0u16; 32768];
                    let mut resource = 0;
                    let mut flags = 0;
                    object.GetOverlayInfo(&mut icon, &mut resource, &mut flags)?;
                    let end = icon
                        .iter()
                        .position(|v| *v == 0)
                        .context("Unterminated icon path")?;
                    anyhow::ensure!(
                        std::path::Path::new(&String::from_utf16(&icon[..end])?).is_file(),
                        "Overlay icon is absent from the installed payload"
                    );
                    anyhow::ensure!(
                        resource == 0 && flags == 3,
                        "Unexpected overlay resource contract"
                    );
                    let missing =
                        omnisync_windows::wide(r"C:\OmniSync-diagnostic-unmanaged-file.txt");
                    let code =
                        (object.vtable().IsMemberOf)(object.as_raw(), PCWSTR(missing.as_ptr()), 0);
                    anyhow::ensure!(
                        code == S_FALSE,
                        "Unmanaged file unexpectedly received an overlay"
                    );
                    anyhow::ensure!(object.GetPriority()? <= 100, "Invalid overlay priority");
                } else {
                    let object: IExplorerCommand = factory.CreateInstance(None)?;
                    let title = object.GetTitle(None)?;
                    anyhow::ensure!(
                        !title.to_string()?.is_empty(),
                        "Empty Explorer command title"
                    );
                    CoTaskMemFree(Some(title.0.cast()));
                    anyhow::ensure!(
                        object.GetState(None, false)? == 1,
                        "Empty selection must disable command"
                    );
                }
            }
            anyhow::ensure!(unload() == S_OK, "COM objects were not released");
            Ok(())
        })();
        FreeLibrary(module)?;
        result
    }
}
