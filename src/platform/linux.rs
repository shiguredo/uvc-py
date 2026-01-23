//! Linux 固有実装 (V4L2)
//!
//! 注: この実装は将来のフェーズで完成させる

use crate::device::Device;
use crate::error::UvcError;
use crate::types::DeviceInfo;

/// デバイス列挙
pub fn list_devices() -> Result<Vec<DeviceInfo>, UvcError> {
    // TODO: V4L2 を使用してデバイスを列挙
    Err(UvcError::PlatformError(
        "Linux サポートは未実装です".to_string(),
    ))
}

/// デバイスをオープン
pub fn open_device(_index: u32) -> Result<Box<dyn Device>, UvcError> {
    // TODO: V4L2 デバイスをオープン
    Err(UvcError::PlatformError(
        "Linux サポートは未実装です".to_string(),
    ))
}
