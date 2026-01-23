//! フレームデータ構造体

use std::ffi::c_void;
use std::sync::Arc;

use numpy::{PyArray1, PyArrayMethods};
use pyo3::IntoPyObjectExt;
use pyo3::prelude::*;
use pyo3::types::PyCapsule;

use crate::types::Format;

/// ネイティブバッファのリリース関数型
pub type NativeBufferReleaseFn = Box<dyn Fn() + Send + Sync>;

/// Send/Sync 可能なポインタラッパー
#[derive(Clone, Copy)]
struct SendPtr(*mut c_void);

// SAFETY: CVPixelBufferRef は参照カウントで管理され、
// 適切にロック/アンロックされるため安全
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

/// フレームの内部データ
pub struct FrameData {
    pub width: u32,
    pub height: u32,
    pub format: Format,
    pub timestamp: u64,

    // NV12 プレーン (ゼロコピー用)
    pub y_plane: Option<*const u8>,
    pub y_stride: usize,
    pub uv_plane: Option<*const u8>,
    pub uv_stride: usize,

    // パックドプレーン (YUY2/RGBA 用)
    pub packed_plane: Option<*const u8>,
    pub packed_stride: usize,

    // ネイティブバッファ
    pub native_buffer: Option<*mut c_void>,
    pub native_buffer_release: Option<NativeBufferReleaseFn>,
}

// SAFETY: FrameData はネイティブバッファのライフタイムを管理し、
// 適切にリリースされる
unsafe impl Send for FrameData {}
unsafe impl Sync for FrameData {}

impl Drop for FrameData {
    fn drop(&mut self) {
        if let Some(release_fn) = self.native_buffer_release.take() {
            release_fn();
        }
    }
}

/// Python に公開するフレーム構造体
#[pyclass]
pub struct Frame {
    data: Arc<FrameData>,
}

impl Frame {
    pub fn new(data: FrameData) -> Self {
        Self {
            data: Arc::new(data),
        }
    }
}

#[pymethods]
impl Frame {
    /// フレーム幅
    #[getter]
    fn width(&self) -> u32 {
        self.data.width
    }

    /// フレーム高さ
    #[getter]
    fn height(&self) -> u32 {
        self.data.height
    }

    /// フレームフォーマット
    #[getter]
    fn format(&self) -> Format {
        self.data.format
    }

    /// タイムスタンプ (マイクロ秒)
    #[getter]
    fn timestamp(&self) -> u64 {
        self.data.timestamp
    }

    /// NV12 Y プレーンと UV プレーンを取得
    fn to_nv12(&self, py: Python<'_>) -> PyResult<(PyObject, PyObject)> {
        if self.data.format != Format::NV12 {
            return Err(pyo3::exceptions::PyRuntimeError::new_err(
                "Frame is not NV12 format",
            ));
        }

        let y_ptr = self
            .data
            .y_plane
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("NV12 Y plane not set"))?;
        let uv_ptr = self
            .data
            .uv_plane
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("NV12 UV plane not set"))?;

        let height = self.data.height as usize;
        let width = self.data.width as usize;
        let y_stride = self.data.y_stride;
        let uv_stride = self.data.uv_stride;

        // Y プレーン: (height, width)
        let y_array = unsafe {
            let y_slice = std::slice::from_raw_parts(y_ptr, height * y_stride);
            // 1 次元配列を作成して reshape
            if y_stride == width {
                let arr = PyArray1::from_slice(py, y_slice);
                arr.reshape([height, width])?.into_py_any(py)?
            } else {
                // ストライドが異なる場合は行ごとにコピー
                let mut y_data = Vec::with_capacity(height * width);
                for row in 0..height {
                    let row_start = row * y_stride;
                    y_data.extend_from_slice(&y_slice[row_start..row_start + width]);
                }
                let arr = PyArray1::from_vec(py, y_data);
                arr.reshape([height, width])?.into_py_any(py)?
            }
        };

        // UV プレーン: (height/2, width)
        let uv_height = height / 2;
        let uv_array = unsafe {
            let uv_slice = std::slice::from_raw_parts(uv_ptr, uv_height * uv_stride);
            if uv_stride == width {
                let arr = PyArray1::from_slice(py, uv_slice);
                arr.reshape([uv_height, width])?.into_py_any(py)?
            } else {
                let mut uv_data = Vec::with_capacity(uv_height * width);
                for row in 0..uv_height {
                    let row_start = row * uv_stride;
                    uv_data.extend_from_slice(&uv_slice[row_start..row_start + width]);
                }
                let arr = PyArray1::from_vec(py, uv_data);
                arr.reshape([uv_height, width])?.into_py_any(py)?
            }
        };

        Ok((y_array, uv_array))
    }

    /// YUY2 データを取得 (H, W, 2)
    fn to_yuy2(&self, py: Python<'_>) -> PyResult<PyObject> {
        if self.data.format != Format::YUY2 {
            return Err(pyo3::exceptions::PyRuntimeError::new_err(
                "Frame is not YUY2 format",
            ));
        }

        let packed_ptr = self
            .data
            .packed_plane
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Packed plane not set"))?;

        let height = self.data.height as usize;
        let width = self.data.width as usize;
        let stride = self.data.packed_stride;
        let row_bytes = width * 2;

        let array = unsafe {
            let slice = std::slice::from_raw_parts(packed_ptr, height * stride);
            if stride == row_bytes {
                let arr = PyArray1::from_slice(py, slice);
                arr.reshape([height, width, 2])?.into_py_any(py)?
            } else {
                let mut data = Vec::with_capacity(height * row_bytes);
                for row in 0..height {
                    let row_start = row * stride;
                    data.extend_from_slice(&slice[row_start..row_start + row_bytes]);
                }
                let arr = PyArray1::from_vec(py, data);
                arr.reshape([height, width, 2])?.into_py_any(py)?
            }
        };

        Ok(array)
    }

    /// RGB データを取得 (H, W, 3)
    fn to_rgb(&self, py: Python<'_>) -> PyResult<PyObject> {
        if self.data.format != Format::RGB {
            return Err(pyo3::exceptions::PyRuntimeError::new_err(
                "Frame is not RGB format",
            ));
        }

        let packed_ptr = self
            .data
            .packed_plane
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Packed plane not set"))?;

        let height = self.data.height as usize;
        let width = self.data.width as usize;
        let stride = self.data.packed_stride;
        let row_bytes = width * 3;

        let array = unsafe {
            let slice = std::slice::from_raw_parts(packed_ptr, height * stride);
            if stride == row_bytes {
                let arr = PyArray1::from_slice(py, slice);
                arr.reshape([height, width, 3])?.into_py_any(py)?
            } else {
                let mut data = Vec::with_capacity(height * row_bytes);
                for row in 0..height {
                    let row_start = row * stride;
                    data.extend_from_slice(&slice[row_start..row_start + row_bytes]);
                }
                let arr = PyArray1::from_vec(py, data);
                arr.reshape([height, width, 3])?.into_py_any(py)?
            }
        };

        Ok(array)
    }

    /// RGBA データを取得 (H, W, 4)
    fn to_rgba(&self, py: Python<'_>) -> PyResult<PyObject> {
        if self.data.format != Format::RGBA {
            return Err(pyo3::exceptions::PyRuntimeError::new_err(
                "Frame is not RGBA format",
            ));
        }

        let packed_ptr = self
            .data
            .packed_plane
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Packed plane not set"))?;

        let height = self.data.height as usize;
        let width = self.data.width as usize;
        let stride = self.data.packed_stride;
        let row_bytes = width * 4;

        let array = unsafe {
            let slice = std::slice::from_raw_parts(packed_ptr, height * stride);
            if stride == row_bytes {
                let arr = PyArray1::from_slice(py, slice);
                arr.reshape([height, width, 4])?.into_py_any(py)?
            } else {
                let mut data = Vec::with_capacity(height * row_bytes);
                for row in 0..height {
                    let row_start = row * stride;
                    data.extend_from_slice(&slice[row_start..row_start + row_bytes]);
                }
                let arr = PyArray1::from_vec(py, data);
                arr.reshape([height, width, 4])?.into_py_any(py)?
            }
        };

        Ok(array)
    }

    /// ネイティブバッファを PyCapsule として取得
    /// macOS: CVPixelBufferRef を "CVPixelBufferRef" という名前の capsule で返す
    /// Linux: None を返す
    fn native_buffer<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyCapsule>>> {
        #[cfg(target_os = "macos")]
        {
            if let Some(buffer) = self.data.native_buffer {
                // CVPixelBufferRef を retain して capsule を作成
                unsafe {
                    crate::platform::macos::cvpixelbuffer_retain(buffer);
                }

                // SendPtr でラップして Send + Sync にする
                let send_buffer = SendPtr(buffer);

                let capsule = PyCapsule::new_with_destructor(
                    py,
                    send_buffer,
                    Some(std::ffi::CString::new("CVPixelBufferRef").unwrap()),
                    |send_ptr, _context| unsafe {
                        crate::platform::macos::cvpixelbuffer_release(send_ptr.0);
                    },
                )?;
                return Ok(Some(capsule));
            }
        }

        #[cfg(not(target_os = "macos"))]
        let _ = py;

        Ok(None)
    }
}
