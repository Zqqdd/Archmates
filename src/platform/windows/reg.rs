//! Small wrappers around the Windows registry API.

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_DWORD, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
    RegDeleteKeyValueW, RegDeleteTreeW, RegGetValueW, RegSetKeyValueW,
};

pub const CURRENT_USER: HKEY = HKEY_CURRENT_USER;
pub const LOCAL_MACHINE: HKEY = HKEY_LOCAL_MACHINE;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn get_string(root: HKEY, key: &str, value: &str) -> Option<String> {
    let (key, value) = (wide(key), wide(value));
    let mut buf = vec![0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let err = unsafe {
        RegGetValueW(
            root,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if err != ERROR_SUCCESS {
        return None;
    }
    let len = (size as usize / 2).min(buf.len());
    let text = String::from_utf16_lossy(&buf[..len]);
    Some(text.trim_end_matches('\0').to_string())
}

pub fn get_dword(root: HKEY, key: &str, value: &str) -> Option<u32> {
    let (key, value) = (wide(key), wide(value));
    let mut out = 0u32;
    let mut size = 4u32;
    let err = unsafe {
        RegGetValueW(
            root,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut out as *mut u32).cast(),
            &mut size,
        )
    };
    (err == ERROR_SUCCESS).then_some(out)
}

pub fn set_string(root: HKEY, key: &str, value: &str, data: &str) -> anyhow::Result<()> {
    let (key, value, data) = (wide(key), wide(value), wide(data));
    let err = unsafe {
        RegSetKeyValueW(root, key.as_ptr(), value.as_ptr(), REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32)
    };
    anyhow::ensure!(err == ERROR_SUCCESS, "could not write the registry (error {err})");
    Ok(())
}

pub fn set_dword(root: HKEY, key: &str, value: &str, data: u32) -> anyhow::Result<()> {
    let (key, value) = (wide(key), wide(value));
    let err =
        unsafe { RegSetKeyValueW(root, key.as_ptr(), value.as_ptr(), REG_DWORD, (&data as *const u32).cast(), 4) };
    anyhow::ensure!(err == ERROR_SUCCESS, "could not write the registry (error {err})");
    Ok(())
}

pub fn delete_tree(root: HKEY, key: &str) {
    let key = wide(key);
    unsafe {
        RegDeleteTreeW(root, key.as_ptr());
    }
}

pub fn delete_value(root: HKEY, key: &str, value: &str) {
    let (key, value) = (wide(key), wide(value));
    unsafe {
        RegDeleteKeyValueW(root, key.as_ptr(), value.as_ptr());
    }
}
