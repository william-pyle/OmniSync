use anyhow::{bail, Result};
use std::path::Path;
use windows::{
    core::{PCWSTR, PWSTR},
    Win32::{
        Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL},
        Security::{Authorization::*, Cryptography::*, *},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    },
};

pub fn sid() -> Result<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
        let mut length = 0;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut length);
        let mut bytes = vec![0u64; (length as usize).div_ceil(8)];
        let result = (|| -> Result<String> {
            GetTokenInformation(
                token,
                TokenUser,
                Some(bytes.as_mut_ptr().cast()),
                length,
                &mut length,
            )?;
            let user = &*(bytes.as_ptr().cast::<TOKEN_USER>());
            let mut text = PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &mut text)?;
            let result = text.to_string()?;
            let _ = LocalFree(Some(HLOCAL(text.0.cast())));
            Ok(result)
        })();
        let _ = CloseHandle(token);
        result
    }
}
pub struct Descriptor(pub PSECURITY_DESCRIPTOR);
impl Descriptor {
    pub fn user() -> Result<Self> {
        Self::from_sddl(&format!("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;{})", sid()?))
    }
    pub fn administrators() -> Result<Self> {
        // Receipts are readable for review; only administrators and SYSTEM can modify them.
        Self::from_sddl("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;BU)")
    }
    pub fn from_sddl(value: &str) -> Result<Self> {
        unsafe {
            let mut ptr = PSECURITY_DESCRIPTOR::default();
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(crate::wide(value).as_ptr()),
                1,
                &mut ptr,
                None,
            )?;
            Ok(Self(ptr))
        }
    }
}
impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.0 .0)));
        }
    }
}
pub fn private_directory(path: &Path, admin: bool) -> Result<()> {
    std::fs::create_dir_all(path)?;
    let descriptor = if admin {
        Descriptor::administrators()?
    } else {
        Descriptor::user()?
    };
    unsafe {
        if !SetFileSecurityW(
            PCWSTR(crate::wide(path).as_ptr()),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor.0,
        )
        .as_bool()
        {
            bail!("Could not protect the OmniSync state directory");
        }
    }
    Ok(())
}
pub fn protect(bytes: &[u8]) -> Result<Vec<u8>> {
    crypt(bytes, true)
}
pub fn unprotect(bytes: &[u8]) -> Result<Vec<u8>> {
    crypt(bytes, false)
}
fn crypt(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: bytes.len().try_into()?,
            pbData: bytes.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        if encrypt {
            CryptProtectData(
                &input,
                PCWSTR(crate::wide("OmniSync user credentials").as_ptr()),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )?;
        } else {
            CryptUnprotectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )?;
        }
        let result = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dpapi_roundtrip() {
        let plain = b"test-only-token";
        let cipher = protect(plain).unwrap();
        assert_ne!(cipher, plain);
        assert_eq!(unprotect(&cipher).unwrap(), plain);
        assert!(sid().unwrap().starts_with("S-1-"));
    }
}
