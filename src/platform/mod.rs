//! プラットフォーム固有実装

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "linux")]
pub mod linux;

use crate::device::Device;
use crate::error::UvcError;
use crate::types::DeviceInfo;

/// デバイス列挙
pub fn list_devices() -> Result<Vec<DeviceInfo>, UvcError> {
    #[cfg(target_os = "macos")]
    {
        macos::list_devices()
    }

    #[cfg(target_os = "linux")]
    {
        linux::list_devices()
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(UvcError::PlatformError(
            "このプラットフォームはサポートされていません".to_string(),
        ))
    }
}

/// デバイスをオープン
pub fn open_device(index: u32) -> Result<Box<dyn Device>, UvcError> {
    #[cfg(target_os = "macos")]
    {
        macos::open_device(index)
    }

    #[cfg(target_os = "linux")]
    {
        linux::open_device(index)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = index;
        Err(UvcError::PlatformError(
            "このプラットフォームはサポートされていません".to_string(),
        ))
    }
}
