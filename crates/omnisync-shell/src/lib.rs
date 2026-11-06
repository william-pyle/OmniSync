//! Explorer calls only bounded shared-memory lookups. Uploads and scans run in OmniSync.
#![allow(non_snake_case)]
use omnisync_windows::{
    integration::{CLSIDS, COMMAND_CLSIDS},
    status::Cache,
};
use std::{
    ffi::c_void,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicIsize, AtomicUsize, Ordering},
        Mutex,
    },
};
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        System::{Com::*, LibraryLoader::*},
        UI::Shell::*,
    },
};
static MODULE: AtomicIsize = AtomicIsize::new(0);
static OBJECTS: AtomicUsize = AtomicUsize::new(0);
static LOCKS: AtomicUsize = AtomicUsize::new(0);
struct Count;
impl Count {
    fn new() -> Self {
        OBJECTS.fetch_add(1, Ordering::SeqCst);
        Self
    }
}
impl Drop for Count {
    fn drop(&mut self) {
        OBJECTS.fetch_sub(1, Ordering::SeqCst);
    }
}
fn module_path() -> Result<PathBuf> {
    unsafe {
        let mut buffer = vec![0u16; 32768];
        let n = GetModuleFileNameW(
            Some(HMODULE(MODULE.load(Ordering::Relaxed) as *mut c_void)),
            &mut buffer,
        );
        if n == 0 || n as usize >= buffer.len() {
            return Err(Error::from_win32());
        }
        Ok(PathBuf::from(String::from_utf16_lossy(
            &buffer[..n as usize],
        )))
    }
}
fn icon_path(name: &str) -> Result<PathBuf> {
    Ok(module_path()?
        .parent()
        .ok_or_else(|| Error::from_hresult(E_FAIL))?
        .join("icons")
        .join(format!("{name}.ico")))
}
fn guid(value: &str) -> GUID {
    GUID::from_u128(
        u128::from_str_radix(&value.trim_matches(['{', '}']).replace('-', ""), 16)
            .expect("constant CLSID"),
    )
}
#[implement(IShellIconOverlayIdentifier)]
struct Overlay {
    state: u32,
    cache: Mutex<Option<Cache>>,
    _count: Count,
}
impl IShellIconOverlayIdentifier_Impl for Overlay_Impl {
    fn IsMemberOf(&self, path: &PCWSTR, _attributes: u32) -> Result<()> {
        if path.is_null() {
            return Err(Error::from_hresult(S_FALSE));
        }
        let value = unsafe {
            let mut length = 0;
            while length < 32768 && *path.0.add(length) != 0 {
                length += 1;
            }
            if length == 32768 {
                return Err(Error::from_hresult(S_FALSE));
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(path.0, length))
        };
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| Error::from_hresult(S_FALSE))?;
        if cache.is_none() {
            *cache = Cache::open(false).ok();
        }
        if cache.as_ref().and_then(|c| c.lookup(Path::new(&value))) == Some(self.state) {
            Ok(())
        } else {
            Err(Error::from_hresult(S_FALSE))
        }
    }
    fn GetOverlayInfo(
        &self,
        path: PWSTR,
        max: i32,
        index: *mut i32,
        flags: *mut u32,
    ) -> Result<()> {
        if path.is_null() || index.is_null() || flags.is_null() || max <= 0 {
            return Err(Error::from_hresult(E_INVALIDARG));
        }
        let names = ["backed", "syncing", "pending", "excluded", "issue"];
        let value = omnisync_windows::wide(icon_path(names[(self.state - 1) as usize])?);
        if value.len() > max as usize {
            return Err(Error::from_hresult(E_FAIL));
        }
        unsafe {
            std::ptr::copy_nonoverlapping(value.as_ptr(), path.0, value.len());
            *index = 0;
            *flags = 3;
        }
        Ok(())
    }
    fn GetPriority(&self) -> Result<i32> {
        Ok(if self.state == 5 { 0 } else { 10 })
    }
}
#[implement(IExplorerCommand)]
struct Command {
    action: usize,
    _count: Count,
}
impl IExplorerCommand_Impl for Command_Impl {
    fn GetTitle(&self, _: Ref<'_, IShellItemArray>) -> Result<PWSTR> {
        unsafe {
            SHStrDupW(PCWSTR(
                omnisync_windows::wide(
                    [
                        "Back up with OmniSync…",
                        "Exclude from OmniSync…",
                        "Show OmniSync status",
                    ][self.action],
                )
                .as_ptr(),
            ))
        }
    }
    fn GetIcon(&self, _: Ref<'_, IShellItemArray>) -> Result<PWSTR> {
        unsafe { SHStrDupW(PCWSTR(omnisync_windows::wide(icon_path("app")?).as_ptr())) }
    }
    fn GetToolTip(&self, items: Ref<'_, IShellItemArray>) -> Result<PWSTR> {
        self.GetTitle(items)
    }
    fn GetCanonicalName(&self) -> Result<GUID> {
        Ok(guid(COMMAND_CLSIDS[self.action]))
    }
    fn GetState(&self, items: Ref<'_, IShellItemArray>, _: BOOL) -> Result<u32> {
        Ok(
            if items.as_ref().and_then(|i| unsafe { i.GetCount().ok() }) == Some(1) {
                0
            } else {
                1
            },
        )
    }
    fn Invoke(&self, items: Ref<'_, IShellItemArray>, _: Ref<'_, IBindCtx>) -> Result<()> {
        unsafe {
            let items = items
                .as_ref()
                .ok_or_else(|| Error::from_hresult(E_INVALIDARG))?;
            if items.GetCount()? != 1 {
                return Err(Error::from_hresult(E_INVALIDARG));
            }
            let item = items.GetItemAt(0)?;
            let value = item.GetDisplayName(SIGDN_FILESYSPATH)?;
            let path = value.to_string()?;
            CoTaskMemFree(Some(value.0.cast()));
            let executable = module_path()?.with_file_name("omnisync.exe");
            std::process::Command::new(executable)
                .arg(["--include", "--exclude", "--status"][self.action])
                .arg(path)
                .creation_flags(0x08000000)
                .spawn()
                .map_err(|_| Error::from_hresult(E_FAIL))?;
        }
        Ok(())
    }
    fn GetFlags(&self) -> Result<u32> {
        Ok(0)
    }
    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        Err(Error::from_hresult(E_NOTIMPL))
    }
}
#[implement(IClassFactory)]
struct Factory {
    kind: usize,
    _count: Count,
}
impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<'_, IUnknown>,
        iid: *const GUID,
        out: *mut *mut c_void,
    ) -> Result<()> {
        if outer.as_ref().is_some() {
            return Err(Error::from_hresult(CLASS_E_NOAGGREGATION));
        }
        if iid.is_null() || out.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        unsafe {
            *out = std::ptr::null_mut();
            let object: IUnknown = if self.kind < 5 {
                Overlay {
                    state: self.kind as u32 + 1,
                    cache: Mutex::new(None),
                    _count: Count::new(),
                }
                .into()
            } else {
                Command {
                    action: self.kind - 5,
                    _count: Count::new(),
                }
                .into()
            };
            object.query(iid, out).ok()
        }
    }
    fn LockServer(&self, lock: BOOL) -> Result<()> {
        if lock.as_bool() {
            LOCKS.fetch_add(1, Ordering::SeqCst);
        } else {
            let _ = LOCKS.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| v.checked_sub(1));
        }
        Ok(())
    }
}
#[no_mangle]
pub unsafe extern "system" fn DllMain(
    module: HMODULE,
    reason: u32,
    _reserved: *mut c_void,
) -> BOOL {
    if reason == 1 {
        MODULE.store(module.0 as isize, Ordering::Relaxed);
    }
    true.into()
}
#[no_mangle]
pub unsafe extern "system" fn DllGetClassObject(
    clsid: *const GUID,
    iid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    if clsid.is_null() || iid.is_null() || out.is_null() {
        return E_POINTER;
    }
    *out = std::ptr::null_mut();
    let kind = CLSIDS
        .iter()
        .chain(COMMAND_CLSIDS.iter())
        .position(|value| guid(value) == *clsid);
    let Some(kind) = kind else {
        return CLASS_E_CLASSNOTAVAILABLE;
    };
    let factory: IClassFactory = Factory {
        kind,
        _count: Count::new(),
    }
    .into();
    factory.query(iid, out)
}
#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if OBJECTS.load(Ordering::SeqCst) == 0 && LOCKS.load(Ordering::SeqCst) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}
