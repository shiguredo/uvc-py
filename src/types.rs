//! UVC 型定義

use pyo3::prelude::*;

/// フォーマット列挙型
#[allow(clippy::upper_case_acronyms)]
#[pyclass(eq, eq_int, frozen, hash)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Format {
    MJPEG,
    YUY2,
    NV12,
    RGB,
    RGBA,
}

#[pymethods]
impl Format {
    fn __repr__(&self) -> String {
        match self {
            Format::MJPEG => "Format.MJPEG".to_string(),
            Format::YUY2 => "Format.YUY2".to_string(),
            Format::NV12 => "Format.NV12".to_string(),
            Format::RGB => "Format.RGB".to_string(),
            Format::RGBA => "Format.RGBA".to_string(),
        }
    }
}

/// デバイス情報
#[pyclass]
#[derive(Clone, Debug)]
pub struct DeviceInfo {
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub unique_id: String,
    #[pyo3(get)]
    pub index: u32,
}

impl DeviceInfo {
    pub fn new(name: String, unique_id: String, index: u32) -> Self {
        Self {
            name,
            unique_id,
            index,
        }
    }
}

#[pymethods]
impl DeviceInfo {
    fn __repr__(&self) -> String {
        format!("DeviceInfo(name='{}', index={})", self.name, self.index)
    }
}

/// フォーマット情報
#[pyclass]
#[derive(Clone, Debug)]
pub struct FormatInfo {
    #[pyo3(get)]
    pub width: u32,
    #[pyo3(get)]
    pub height: u32,
    #[pyo3(get)]
    pub fps: u32,
    #[pyo3(get)]
    pub format: Format,
}

impl FormatInfo {
    pub fn new(width: u32, height: u32, fps: u32, format: Format) -> Self {
        Self {
            width,
            height,
            fps,
            format,
        }
    }
}

#[pymethods]
impl FormatInfo {
    fn __repr__(&self) -> String {
        let fmt_str = match self.format {
            Format::MJPEG => "MJPEG",
            Format::YUY2 => "YUY2",
            Format::NV12 => "NV12",
            Format::RGB => "RGB",
            Format::RGBA => "RGBA",
        };
        format!(
            "{}x{}@{}fps ({})",
            self.width, self.height, self.fps, fmt_str
        )
    }
}
