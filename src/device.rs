//! デバイス抽象化とラッパー

use std::sync::{Arc, Mutex};

use pyo3::prelude::*;

use crate::error::UvcError;
use crate::frame::Frame;
use crate::types::{DeviceInfo, Format, FormatInfo};

/// デバイスコールバック型
pub type DeviceCallback = Box<dyn Fn() + Send + Sync>;

/// デバイストレイト
pub trait Device: Send + Sync {
    /// キャプチャを開始
    fn start(
        &mut self,
        width: u32,
        height: u32,
        fps: u32,
        capture_format: Format,
        output_format: Option<Format>,
    ) -> Result<(), UvcError>;

    /// キャプチャを停止
    fn stop(&mut self) -> Result<(), UvcError>;

    /// フレームを取得
    fn get_frame(&self) -> Option<Frame>;

    /// 実行中かどうか
    fn is_running(&self) -> bool;

    /// デバイス情報を取得
    fn info(&self) -> &DeviceInfo;

    /// サポートされているフォーマットを取得
    fn get_supported_formats(&self) -> Vec<FormatInfo>;

    /// 接続時コールバックを設定
    fn set_on_connected(&mut self, callback: Option<DeviceCallback>);

    /// 切断時コールバックを設定
    fn set_on_disconnected(&mut self, callback: Option<DeviceCallback>);
}

/// Python に公開するデバイスラッパー
#[pyclass]
pub struct PyDevice {
    inner: Arc<Mutex<Box<dyn Device>>>,
}

impl PyDevice {
    pub fn new(device: Box<dyn Device>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(device)),
        }
    }
}

#[pymethods]
impl PyDevice {
    /// キャプチャを開始
    #[pyo3(signature = (width, height, fps, capture_format=Format::MJPEG, output_format=None))]
    fn start(
        &self,
        width: u32,
        height: u32,
        fps: u32,
        capture_format: Format,
        output_format: Option<Format>,
    ) -> PyResult<()> {
        let mut device = self.inner.lock().unwrap();
        device.start(width, height, fps, capture_format, output_format)?;
        Ok(())
    }

    /// キャプチャを停止
    fn stop(&self) -> PyResult<()> {
        let mut device = self.inner.lock().unwrap();
        device.stop()?;
        Ok(())
    }

    /// フレームを取得
    fn get_frame(&self, py: Python<'_>) -> Option<Frame> {
        // GIL を解放してフレーム取得
        let inner = self.inner.clone();
        py.allow_threads(|| {
            let device = inner.lock().unwrap();
            device.get_frame()
        })
    }

    /// 実行中かどうか
    #[getter]
    fn is_running(&self) -> bool {
        let device = self.inner.lock().unwrap();
        device.is_running()
    }

    /// デバイス情報を取得
    #[getter]
    fn info(&self) -> DeviceInfo {
        let device = self.inner.lock().unwrap();
        device.info().clone()
    }

    /// サポートされているフォーマットを取得
    fn get_supported_formats(&self) -> Vec<FormatInfo> {
        let device = self.inner.lock().unwrap();
        device.get_supported_formats()
    }

    /// コンテキストマネージャー: __enter__
    fn __enter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    /// コンテキストマネージャー: __exit__
    fn __exit__(
        &self,
        _exc_type: Option<&Bound<'_, pyo3::types::PyAny>>,
        _exc_value: Option<&Bound<'_, pyo3::types::PyAny>>,
        _traceback: Option<&Bound<'_, pyo3::types::PyAny>>,
    ) -> PyResult<bool> {
        let mut device = self.inner.lock().unwrap();
        if device.is_running() {
            let _ = device.stop();
        }
        // 例外を抑制しない
        Ok(false)
    }
}

/// コールバックを設定するための内部メソッド
impl PyDevice {
    pub fn set_on_connected_internal(&self, callback: Option<DeviceCallback>) {
        let mut device = self.inner.lock().unwrap();
        device.set_on_connected(callback);
    }

    pub fn set_on_disconnected_internal(&self, callback: Option<DeviceCallback>) {
        let mut device = self.inner.lock().unwrap();
        device.set_on_disconnected(callback);
    }
}
