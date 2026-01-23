//! UVC カメラライブラリ (Rust + PyO3 実装)

use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;

mod device;
mod error;
mod frame;
pub mod platform;
mod types;

use device::PyDevice;
use frame::Frame;
use types::{DeviceInfo, Format, FormatInfo};

/// UVC モジュール
#[pymodule(gil_used = false)]
fn uvc_ext(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Format>()?;
    m.add_class::<DeviceInfo>()?;
    m.add_class::<FormatInfo>()?;
    m.add_class::<Frame>()?;
    m.add_class::<PyDevice>()?;
    m.add_function(wrap_pyfunction!(list_devices, m)?)?;
    m.add_function(wrap_pyfunction!(open, m)?)?;
    Ok(())
}

/// 利用可能な UVC デバイスを列挙する
#[pyfunction]
fn list_devices() -> PyResult<Vec<DeviceInfo>> {
    platform::list_devices().map_err(PyErr::from)
}

/// デバイスをオープンする
///
/// # 引数
/// - `index_or_info`: デバイスインデックス (u32) または DeviceInfo
/// - `on_connected`: デバイス接続時のコールバック (オプション)
/// - `on_disconnected`: デバイス切断時のコールバック (オプション)
#[pyfunction]
#[pyo3(signature = (index_or_info, on_connected=None, on_disconnected=None))]
fn open(
    py: Python<'_>,
    index_or_info: &Bound<'_, pyo3::types::PyAny>,
    on_connected: Option<PyObject>,
    on_disconnected: Option<PyObject>,
) -> PyResult<PyDevice> {
    // index または DeviceInfo を判定
    let index = if let Ok(idx) = index_or_info.extract::<u32>() {
        idx
    } else if let Ok(info) = index_or_info.extract::<DeviceInfo>() {
        info.index
    } else {
        return Err(PyTypeError::new_err("index または DeviceInfo が必要です"));
    };

    let device = py.allow_threads(|| platform::open_device(index))?;
    let py_device = PyDevice::new(device);

    // コールバックを設定
    if let Some(callback) = on_connected {
        let callback_clone = callback.clone_ref(py);
        py_device.set_on_connected_internal(Some(Box::new(move || {
            Python::with_gil(|py| {
                if let Err(e) = callback_clone.call0(py) {
                    e.print(py);
                }
            });
        })));
    }

    if let Some(callback) = on_disconnected {
        let callback_clone = callback.clone_ref(py);
        py_device.set_on_disconnected_internal(Some(Box::new(move || {
            Python::with_gil(|py| {
                if let Err(e) = callback_clone.call0(py) {
                    e.print(py);
                }
            });
        })));
    }

    Ok(py_device)
}
