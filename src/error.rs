//! UVC エラー型定義

use std::fmt;

use pyo3::PyErr;
use pyo3::exceptions::PyRuntimeError;

/// UVC エラー型
#[derive(Debug)]
pub enum UvcError {
    /// デバイスが見つからない
    DeviceNotFound(u32),
    /// キャプチャエラー
    CaptureError(String),
    /// フォーマットエラー
    FormatError(String),
    /// デバイスが既に実行中
    AlreadyRunning,
    /// デバイスが実行中でない
    NotRunning,
    /// プラットフォーム固有エラー
    PlatformError(String),
}

impl fmt::Display for UvcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UvcError::DeviceNotFound(index) => {
                write!(f, "デバイスが見つかりません: {}", index)
            }
            UvcError::CaptureError(msg) => {
                write!(f, "キャプチャエラー: {}", msg)
            }
            UvcError::FormatError(msg) => {
                write!(f, "フォーマットエラー: {}", msg)
            }
            UvcError::AlreadyRunning => {
                write!(f, "デバイスは既に実行中です")
            }
            UvcError::NotRunning => {
                write!(f, "デバイスは実行中ではありません")
            }
            UvcError::PlatformError(msg) => {
                write!(f, "プラットフォームエラー: {}", msg)
            }
        }
    }
}

impl std::error::Error for UvcError {}

impl From<UvcError> for PyErr {
    fn from(err: UvcError) -> PyErr {
        PyRuntimeError::new_err(err.to_string())
    }
}
