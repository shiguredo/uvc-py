//! Linux 固有実装 (V4L2)
//!
//! libc のみを使用した V4L2 実装

use std::collections::HashMap;
use std::ffi::CString;
use std::sync::{Arc, Mutex, RwLock};

use crate::device::{Device, DeviceCallback};
use crate::error::UvcError;
use crate::frame::{Frame, FrameData, NativeBufferReleaseFn};
use crate::types::{DeviceInfo, Format, FormatInfo};

// ============================================================================
// V4L2 定数
// ============================================================================

// ioctl コマンド番号
const VIDIOC_QUERYCAP: libc::c_ulong = 0x80685600;
const VIDIOC_ENUM_FMT: libc::c_ulong = 0xC0405602;
const VIDIOC_S_FMT: libc::c_ulong = 0xC0D05605;
const VIDIOC_REQBUFS: libc::c_ulong = 0xC0145608;
const VIDIOC_QUERYBUF: libc::c_ulong = 0xC0445609;
const VIDIOC_QBUF: libc::c_ulong = 0xC044560F;
const VIDIOC_DQBUF: libc::c_ulong = 0xC0445611;
const VIDIOC_STREAMON: libc::c_ulong = 0x40045612;
const VIDIOC_STREAMOFF: libc::c_ulong = 0x40045613;
const VIDIOC_ENUM_FRAMESIZES: libc::c_ulong = 0xC02C564A;
const VIDIOC_ENUM_FRAMEINTERVALS: libc::c_ulong = 0xC034564B;

// V4L2 ケーパビリティフラグ
const V4L2_CAP_VIDEO_CAPTURE: u32 = 0x00000001;
const V4L2_CAP_STREAMING: u32 = 0x04000000;

// V4L2 バッファタイプ
const V4L2_BUF_TYPE_VIDEO_CAPTURE: u32 = 1;

// V4L2 メモリタイプ
const V4L2_MEMORY_MMAP: u32 = 1;

// V4L2 フィールド
const V4L2_FIELD_NONE: u32 = 1;

// V4L2 ピクセルフォーマット (FourCC)
const V4L2_PIX_FMT_YUYV: u32 = 0x56595559; // 'YUYV'
const V4L2_PIX_FMT_MJPEG: u32 = 0x47504A4D; // 'MJPG'
const V4L2_PIX_FMT_NV12: u32 = 0x3231564E; // 'NV12'
const V4L2_PIX_FMT_RGB24: u32 = 0x33424752; // 'RGB3'
const V4L2_PIX_FMT_BGR24: u32 = 0x33524742; // 'BGR3'

// フレームサイズタイプ
const V4L2_FRMSIZE_TYPE_DISCRETE: u32 = 1;

// フレームインターバルタイプ
const V4L2_FRMIVAL_TYPE_DISCRETE: u32 = 1;

// バッファ数
const BUFFER_COUNT: u32 = 4;

// ============================================================================
// V4L2 構造体
// ============================================================================

#[repr(C)]
struct V4l2Capability {
    driver: [u8; 16],
    card: [u8; 32],
    bus_info: [u8; 32],
    version: u32,
    capabilities: u32,
    device_caps: u32,
    reserved: [u32; 3],
}

#[repr(C)]
struct V4l2Fmtdesc {
    index: u32,
    buf_type: u32,
    flags: u32,
    description: [u8; 32],
    pixelformat: u32,
    mbus_code: u32,
    reserved: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct V4l2PixFormat {
    width: u32,
    height: u32,
    pixelformat: u32,
    field: u32,
    bytesperline: u32,
    sizeimage: u32,
    colorspace: u32,
    priv_: u32,
    flags: u32,
    ycbcr_enc_or_hsv_enc: u32,
    quantization: u32,
    xfer_func: u32,
}

#[repr(C)]
union V4l2FormatUnion {
    pix: V4l2PixFormat,
    raw_data: [u8; 200],
}

#[repr(C)]
struct V4l2Format {
    buf_type: u32,
    fmt: V4l2FormatUnion,
}

#[repr(C)]
struct V4l2Requestbuffers {
    count: u32,
    buf_type: u32,
    memory: u32,
    capabilities: u32,
    flags: u8,
    reserved: [u8; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct V4l2Timecode {
    tc_type: u32,
    flags: u32,
    frames: u8,
    seconds: u8,
    minutes: u8,
    hours: u8,
    userbits: [u8; 4],
}

#[repr(C)]
union V4l2BufferUnion {
    offset: u32,
    userptr: libc::c_ulong,
    planes: *mut libc::c_void,
    fd: i32,
}

#[repr(C)]
struct V4l2Buffer {
    index: u32,
    buf_type: u32,
    bytesused: u32,
    flags: u32,
    field: u32,
    timestamp: libc::timeval,
    timecode: V4l2Timecode,
    sequence: u32,
    memory: u32,
    m: V4l2BufferUnion,
    length: u32,
    reserved2: u32,
    request_fd_or_reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct V4l2FrmsizeDiscrete {
    width: u32,
    height: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct V4l2FrmsizeStepwise {
    min_width: u32,
    max_width: u32,
    step_width: u32,
    min_height: u32,
    max_height: u32,
    step_height: u32,
}

#[repr(C)]
union V4l2FrmsizeUnion {
    discrete: V4l2FrmsizeDiscrete,
    stepwise: V4l2FrmsizeStepwise,
}

#[repr(C)]
struct V4l2Frmsizeenum {
    index: u32,
    pixel_format: u32,
    size_type: u32,
    frame_size: V4l2FrmsizeUnion,
    reserved: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct V4l2Fract {
    numerator: u32,
    denominator: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct V4l2FrmivalDiscrete {
    numerator: u32,
    denominator: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct V4l2FrmivalStepwise {
    min: V4l2Fract,
    max: V4l2Fract,
    step: V4l2Fract,
}

#[repr(C)]
union V4l2FrmivalUnion {
    discrete: V4l2FrmivalDiscrete,
    stepwise: V4l2FrmivalStepwise,
}

#[repr(C)]
struct V4l2Frmivalenum {
    index: u32,
    pixel_format: u32,
    width: u32,
    height: u32,
    interval_type: u32,
    frame_interval: V4l2FrmivalUnion,
    reserved: [u32; 2],
}

// ============================================================================
// ヘルパー関数
// ============================================================================

/// ioctl を実行
unsafe fn v4l2_ioctl(fd: i32, request: libc::c_ulong, arg: *mut libc::c_void) -> i32 {
    loop {
        let result = unsafe { libc::ioctl(fd, request, arg) };
        if result == -1 && unsafe { *libc::__errno_location() } == libc::EINTR {
            continue;
        }
        return result;
    }
}

/// V4L2 ピクセルフォーマットから Format へ変換
fn pixel_format_to_format(pixel_format: u32) -> Option<Format> {
    match pixel_format {
        V4L2_PIX_FMT_YUYV => Some(Format::YUY2),
        V4L2_PIX_FMT_MJPEG => Some(Format::MJPEG),
        V4L2_PIX_FMT_NV12 => Some(Format::NV12),
        V4L2_PIX_FMT_RGB24 | V4L2_PIX_FMT_BGR24 => Some(Format::RGB),
        _ => None,
    }
}

/// Format から V4L2 ピクセルフォーマットへ変換
fn format_to_pixel_format(format: Format) -> u32 {
    match format {
        Format::YUY2 => V4L2_PIX_FMT_YUYV,
        Format::MJPEG => V4L2_PIX_FMT_MJPEG,
        Format::NV12 => V4L2_PIX_FMT_NV12,
        Format::RGB | Format::RGBA => V4L2_PIX_FMT_RGB24,
    }
}

/// NULL 終端バイト配列から文字列を取得
fn bytes_to_string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).to_string()
}

/// デバイスがビデオキャプチャ対応かチェック
fn is_video_capture_device(fd: i32) -> bool {
    unsafe {
        let mut cap: V4l2Capability = std::mem::zeroed();
        if v4l2_ioctl(fd, VIDIOC_QUERYCAP, &mut cap as *mut _ as *mut libc::c_void) == -1 {
            return false;
        }
        (cap.capabilities & V4L2_CAP_VIDEO_CAPTURE) != 0
            && (cap.capabilities & V4L2_CAP_STREAMING) != 0
    }
}

/// デバイス情報を取得
fn get_device_info(fd: i32, index: u32, device_path: &str) -> Option<DeviceInfo> {
    unsafe {
        let mut cap: V4l2Capability = std::mem::zeroed();
        if v4l2_ioctl(fd, VIDIOC_QUERYCAP, &mut cap as *mut _ as *mut libc::c_void) == -1 {
            return None;
        }
        let name = bytes_to_string(&cap.card);
        let unique_id = device_path.to_string();
        Some(DeviceInfo::new(name, unique_id, index))
    }
}

// ============================================================================
// 公開 API: デバイス列挙
// ============================================================================

/// 利用可能なカメラデバイスを列挙
pub fn list_devices() -> Result<Vec<DeviceInfo>, UvcError> {
    let mut devices = Vec::new();
    let mut index = 0u32;

    for i in 0..64 {
        let path = format!("/dev/video{}", i);
        let c_path = match CString::new(path.clone()) {
            Ok(p) => p,
            Err(_) => continue,
        };

        unsafe {
            let fd = libc::open(c_path.as_ptr(), libc::O_RDWR | libc::O_NONBLOCK);
            if fd == -1 {
                continue;
            }

            if is_video_capture_device(fd) {
                if let Some(info) = get_device_info(fd, index, &path) {
                    devices.push(info);
                    index += 1;
                }
            }

            libc::close(fd);
        }
    }

    Ok(devices)
}

// ============================================================================
// 公開 API: デバイスオープン
// ============================================================================

/// インデックスを指定してデバイスをオープン
pub fn open_device(index: u32) -> Result<Box<dyn Device>, UvcError> {
    let devices = list_devices()?;

    if index as usize >= devices.len() {
        return Err(UvcError::DeviceNotFound(index));
    }

    let info = devices[index as usize].clone();
    let device_path = info.unique_id.clone();

    Ok(Box::new(DeviceLinux::new(info, device_path)))
}

// ============================================================================
// バッファ管理
// ============================================================================

struct MappedBuffer {
    ptr: *mut u8,
    length: usize,
}

// SAFETY: MappedBuffer は mmap されたメモリへのポインタを保持
// デバイスが適切に管理される限り安全
unsafe impl Send for MappedBuffer {}
unsafe impl Sync for MappedBuffer {}

// ============================================================================
// DeviceLinux 実装
// ============================================================================

/// Linux デバイス実装
pub struct DeviceLinux {
    info: DeviceInfo,
    device_path: String,
    fd: Arc<Mutex<Option<i32>>>,
    running: Arc<RwLock<bool>>,
    buffers: Arc<Mutex<Vec<MappedBuffer>>>,
    latest_frame: Arc<Mutex<Option<Frame>>>,
    capture_thread: Arc<Mutex<Option<std::thread::JoinHandle<()>>>>,
    stop_flag: Arc<RwLock<bool>>,
    current_format: Arc<Mutex<Option<(u32, u32, Format)>>>,
    on_connected: Arc<Mutex<Option<DeviceCallback>>>,
    on_disconnected: Arc<Mutex<Option<DeviceCallback>>>,
}

unsafe impl Send for DeviceLinux {}
unsafe impl Sync for DeviceLinux {}

impl DeviceLinux {
    pub fn new(info: DeviceInfo, device_path: String) -> Self {
        Self {
            info,
            device_path,
            fd: Arc::new(Mutex::new(None)),
            running: Arc::new(RwLock::new(false)),
            buffers: Arc::new(Mutex::new(Vec::new())),
            latest_frame: Arc::new(Mutex::new(None)),
            capture_thread: Arc::new(Mutex::new(None)),
            stop_flag: Arc::new(RwLock::new(false)),
            current_format: Arc::new(Mutex::new(None)),
            on_connected: Arc::new(Mutex::new(None)),
            on_disconnected: Arc::new(Mutex::new(None)),
        }
    }

    /// バッファをセットアップ
    fn setup_buffers(&self, fd: i32) -> Result<(), UvcError> {
        unsafe {
            // バッファを要求
            let mut req: V4l2Requestbuffers = std::mem::zeroed();
            req.count = BUFFER_COUNT;
            req.buf_type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
            req.memory = V4L2_MEMORY_MMAP;

            if v4l2_ioctl(fd, VIDIOC_REQBUFS, &mut req as *mut _ as *mut libc::c_void) == -1 {
                return Err(UvcError::PlatformError(
                    "バッファの要求に失敗しました".to_string(),
                ));
            }

            let mut buffers_guard = self.buffers.lock().unwrap();

            for i in 0..req.count {
                let mut buf: V4l2Buffer = std::mem::zeroed();
                buf.buf_type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
                buf.memory = V4L2_MEMORY_MMAP;
                buf.index = i;

                if v4l2_ioctl(fd, VIDIOC_QUERYBUF, &mut buf as *mut _ as *mut libc::c_void) == -1 {
                    return Err(UvcError::PlatformError(format!(
                        "バッファ {} のクエリに失敗しました",
                        i
                    )));
                }

                let ptr = libc::mmap(
                    std::ptr::null_mut(),
                    buf.length as usize,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    fd,
                    buf.m.offset as libc::off_t,
                );

                if ptr == libc::MAP_FAILED {
                    return Err(UvcError::PlatformError(format!(
                        "バッファ {} の mmap に失敗しました",
                        i
                    )));
                }

                buffers_guard.push(MappedBuffer {
                    ptr: ptr as *mut u8,
                    length: buf.length as usize,
                });
            }

            Ok(())
        }
    }

    /// バッファをキューに入れる
    fn queue_buffers(&self, fd: i32) -> Result<(), UvcError> {
        unsafe {
            let buffers_guard = self.buffers.lock().unwrap();
            for i in 0..buffers_guard.len() {
                let mut buf: V4l2Buffer = std::mem::zeroed();
                buf.buf_type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
                buf.memory = V4L2_MEMORY_MMAP;
                buf.index = i as u32;

                if v4l2_ioctl(fd, VIDIOC_QBUF, &mut buf as *mut _ as *mut libc::c_void) == -1 {
                    return Err(UvcError::PlatformError(format!(
                        "バッファ {} のキューイングに失敗しました",
                        i
                    )));
                }
            }
            Ok(())
        }
    }

    /// バッファを解放
    fn cleanup_buffers(&self, fd: i32) {
        let mut buffers_guard = self.buffers.lock().unwrap();
        for buffer in buffers_guard.drain(..) {
            unsafe {
                libc::munmap(buffer.ptr as *mut libc::c_void, buffer.length);
            }
        }

        unsafe {
            let mut req: V4l2Requestbuffers = std::mem::zeroed();
            req.count = 0;
            req.buf_type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
            req.memory = V4L2_MEMORY_MMAP;
            v4l2_ioctl(fd, VIDIOC_REQBUFS, &mut req as *mut _ as *mut libc::c_void);
        }
    }

    /// キャプチャループ
    fn capture_loop(
        fd: i32,
        stop_flag: Arc<RwLock<bool>>,
        latest_frame: Arc<Mutex<Option<Frame>>>,
        buffers: Arc<Mutex<Vec<MappedBuffer>>>,
        format_info: (u32, u32, Format),
    ) {
        let (width, height, format) = format_info;

        loop {
            {
                let stop = stop_flag.read().unwrap();
                if *stop {
                    break;
                }
            }

            unsafe {
                // poll でフレーム待ち
                let mut fds = libc::pollfd {
                    fd,
                    events: libc::POLLIN,
                    revents: 0,
                };

                let poll_result = libc::poll(&mut fds, 1, 100);
                if poll_result <= 0 {
                    continue;
                }

                // バッファをデキュー
                let mut buf: V4l2Buffer = std::mem::zeroed();
                buf.buf_type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
                buf.memory = V4L2_MEMORY_MMAP;

                if v4l2_ioctl(fd, VIDIOC_DQBUF, &mut buf as *mut _ as *mut libc::c_void) == -1 {
                    continue;
                }

                let buffers_guard = buffers.lock().unwrap();
                if (buf.index as usize) < buffers_guard.len() {
                    let mapped_buffer = &buffers_guard[buf.index as usize];

                    // タイムスタンプをマイクロ秒に変換
                    let timestamp =
                        (buf.timestamp.tv_sec as u64) * 1_000_000 + (buf.timestamp.tv_usec as u64);

                    // フレームデータをコピー
                    let data_len = buf.bytesused as usize;
                    let mut frame_buffer = vec![0u8; data_len];
                    std::ptr::copy_nonoverlapping(
                        mapped_buffer.ptr,
                        frame_buffer.as_mut_ptr(),
                        data_len,
                    );

                    drop(buffers_guard);

                    // Frame を作成
                    let frame = Self::create_frame(frame_buffer, width, height, format, timestamp);

                    // 最新フレームを更新
                    if let Ok(mut guard) = latest_frame.lock() {
                        *guard = Some(frame);
                    }
                } else {
                    drop(buffers_guard);
                }

                // バッファを再キュー
                v4l2_ioctl(fd, VIDIOC_QBUF, &mut buf as *mut _ as *mut libc::c_void);
            }
        }
    }

    /// Frame を作成
    fn create_frame(
        data: Vec<u8>,
        width: u32,
        height: u32,
        format: Format,
        timestamp: u64,
    ) -> Frame {
        let data = Arc::new(data);
        let data_clone = data.clone();

        let release_fn: NativeBufferReleaseFn = Box::new(move || {
            // data_clone を所有してドロップ
            let _ = data_clone.len();
        });

        let data_ref = &*data;

        let mut frame_data = FrameData {
            width,
            height,
            format,
            timestamp,
            y_plane: None,
            y_stride: 0,
            uv_plane: None,
            uv_stride: 0,
            packed_plane: None,
            packed_stride: 0,
            native_buffer: None,
            native_buffer_release: Some(release_fn),
        };

        match format {
            Format::NV12 => {
                let y_size = (width * height) as usize;
                frame_data.y_plane = Some(data_ref.as_ptr() as *mut u8);
                frame_data.y_stride = width as usize;
                if data_ref.len() > y_size {
                    frame_data.uv_plane = Some(unsafe { data_ref.as_ptr().add(y_size) as *mut u8 });
                    frame_data.uv_stride = width as usize;
                }
            }
            Format::YUY2 | Format::RGB | Format::RGBA | Format::MJPEG => {
                frame_data.packed_plane = Some(data_ref.as_ptr() as *mut u8);
                frame_data.packed_stride = match format {
                    Format::YUY2 => (width * 2) as usize,
                    Format::RGB => (width * 3) as usize,
                    Format::RGBA => (width * 4) as usize,
                    Format::MJPEG => data_ref.len(),
                    _ => 0,
                };
            }
        }

        Frame::new(frame_data)
    }
}

impl Device for DeviceLinux {
    fn start(
        &mut self,
        width: u32,
        height: u32,
        _fps: u32,
        capture_format: Format,
        _output_format: Option<Format>,
    ) -> Result<(), UvcError> {
        {
            let running = self.running.read().unwrap();
            if *running {
                return Err(UvcError::AlreadyRunning);
            }
        }

        let c_path = CString::new(self.device_path.clone())
            .map_err(|_| UvcError::PlatformError("無効なデバイスパス".to_string()))?;

        unsafe {
            // デバイスをオープン
            let fd = libc::open(c_path.as_ptr(), libc::O_RDWR);
            if fd == -1 {
                return Err(UvcError::PlatformError(
                    "デバイスのオープンに失敗しました".to_string(),
                ));
            }

            // フォーマットを設定
            let mut fmt: V4l2Format = std::mem::zeroed();
            fmt.buf_type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
            fmt.fmt.pix.width = width;
            fmt.fmt.pix.height = height;
            fmt.fmt.pix.pixelformat = format_to_pixel_format(capture_format);
            fmt.fmt.pix.field = V4L2_FIELD_NONE;

            if v4l2_ioctl(fd, VIDIOC_S_FMT, &mut fmt as *mut _ as *mut libc::c_void) == -1 {
                libc::close(fd);
                return Err(UvcError::FormatError(format!(
                    "フォーマット設定に失敗しました: {}x{} {:?}",
                    width, height, capture_format
                )));
            }

            // 実際に設定されたフォーマットを確認
            let actual_width = fmt.fmt.pix.width;
            let actual_height = fmt.fmt.pix.height;

            {
                let mut fd_guard = self.fd.lock().unwrap();
                *fd_guard = Some(fd);
            }

            // バッファをセットアップ
            self.setup_buffers(fd)?;
            self.queue_buffers(fd)?;

            // ストリーム開始
            let buf_type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
            if v4l2_ioctl(
                fd,
                VIDIOC_STREAMON,
                &buf_type as *const _ as *mut libc::c_void,
            ) == -1
            {
                self.cleanup_buffers(fd);
                libc::close(fd);
                let mut fd_guard = self.fd.lock().unwrap();
                *fd_guard = None;
                return Err(UvcError::PlatformError(
                    "ストリームの開始に失敗しました".to_string(),
                ));
            }

            // フォーマット情報を保存
            {
                let mut format_guard = self.current_format.lock().unwrap();
                *format_guard = Some((actual_width, actual_height, capture_format));
            }

            // 停止フラグをリセット
            {
                let mut stop = self.stop_flag.write().unwrap();
                *stop = false;
            }

            // キャプチャスレッドを開始
            let stop_flag = self.stop_flag.clone();
            let latest_frame = self.latest_frame.clone();
            let buffers = self.buffers.clone();
            let format_info = (actual_width, actual_height, capture_format);

            let handle = std::thread::spawn(move || {
                Self::capture_loop(fd, stop_flag, latest_frame, buffers, format_info);
            });

            {
                let mut thread_guard = self.capture_thread.lock().unwrap();
                *thread_guard = Some(handle);
            }

            {
                let mut running = self.running.write().unwrap();
                *running = true;
            }
        }

        Ok(())
    }

    fn stop(&mut self) -> Result<(), UvcError> {
        {
            let running = self.running.read().unwrap();
            if !*running {
                return Ok(());
            }
        }

        // 停止フラグを設定
        {
            let mut stop = self.stop_flag.write().unwrap();
            *stop = true;
        }

        // スレッドの終了を待機
        {
            let mut thread_guard = self.capture_thread.lock().unwrap();
            if let Some(handle) = thread_guard.take() {
                let _ = handle.join();
            }
        }

        let fd_opt = {
            let mut fd_guard = self.fd.lock().unwrap();
            fd_guard.take()
        };

        if let Some(fd) = fd_opt {
            unsafe {
                // ストリーム停止
                let buf_type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
                v4l2_ioctl(
                    fd,
                    VIDIOC_STREAMOFF,
                    &buf_type as *const _ as *mut libc::c_void,
                );

                // バッファ解放
                self.cleanup_buffers(fd);

                // デバイスをクローズ
                libc::close(fd);
            }
        }

        {
            let mut running = self.running.write().unwrap();
            *running = false;
        }

        {
            let mut frame = self.latest_frame.lock().unwrap();
            *frame = None;
        }

        Ok(())
    }

    fn get_frame(&self) -> Option<Frame> {
        let mut frame_guard = self.latest_frame.lock().unwrap();
        frame_guard.take()
    }

    fn is_running(&self) -> bool {
        let running = self.running.read().unwrap();
        *running
    }

    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    fn get_supported_formats(&self) -> Vec<FormatInfo> {
        let c_path = match CString::new(self.device_path.clone()) {
            Ok(p) => p,
            Err(_) => return Vec::new(),
        };

        unsafe {
            let fd = libc::open(c_path.as_ptr(), libc::O_RDWR);
            if fd == -1 {
                return Vec::new();
            }

            let mut formats = Vec::new();
            let mut format_map: HashMap<u32, Vec<(u32, u32, Vec<u32>)>> = HashMap::new();

            // フォーマットを列挙
            let mut fmt_index = 0u32;
            loop {
                let mut fmtdesc: V4l2Fmtdesc = std::mem::zeroed();
                fmtdesc.index = fmt_index;
                fmtdesc.buf_type = V4L2_BUF_TYPE_VIDEO_CAPTURE;

                if v4l2_ioctl(
                    fd,
                    VIDIOC_ENUM_FMT,
                    &mut fmtdesc as *mut _ as *mut libc::c_void,
                ) == -1
                {
                    break;
                }

                let pixelformat = fmtdesc.pixelformat;

                // フレームサイズを列挙
                let mut size_index = 0u32;
                loop {
                    let mut frmsize: V4l2Frmsizeenum = std::mem::zeroed();
                    frmsize.index = size_index;
                    frmsize.pixel_format = pixelformat;

                    if v4l2_ioctl(
                        fd,
                        VIDIOC_ENUM_FRAMESIZES,
                        &mut frmsize as *mut _ as *mut libc::c_void,
                    ) == -1
                    {
                        break;
                    }

                    if frmsize.size_type == V4L2_FRMSIZE_TYPE_DISCRETE {
                        let width = frmsize.frame_size.discrete.width;
                        let height = frmsize.frame_size.discrete.height;

                        // フレームレートを列挙
                        let mut fps_list = Vec::new();
                        let mut interval_index = 0u32;
                        loop {
                            let mut frmival: V4l2Frmivalenum = std::mem::zeroed();
                            frmival.index = interval_index;
                            frmival.pixel_format = pixelformat;
                            frmival.width = width;
                            frmival.height = height;

                            if v4l2_ioctl(
                                fd,
                                VIDIOC_ENUM_FRAMEINTERVALS,
                                &mut frmival as *mut _ as *mut libc::c_void,
                            ) == -1
                            {
                                break;
                            }

                            if frmival.interval_type == V4L2_FRMIVAL_TYPE_DISCRETE {
                                let num = frmival.frame_interval.discrete.numerator;
                                let den = frmival.frame_interval.discrete.denominator;
                                if num > 0 {
                                    fps_list.push(den / num);
                                }
                            }

                            interval_index += 1;
                        }

                        if fps_list.is_empty() {
                            fps_list.push(30);
                        }

                        format_map
                            .entry(pixelformat)
                            .or_default()
                            .push((width, height, fps_list));
                    }

                    size_index += 1;
                }

                fmt_index += 1;
            }

            libc::close(fd);

            // FormatInfo に変換
            for (pixelformat, sizes) in format_map {
                if let Some(format) = pixel_format_to_format(pixelformat) {
                    for (width, height, fps_list) in sizes {
                        for fps in fps_list {
                            formats.push(FormatInfo::new(width, height, fps, format));
                        }
                    }
                }
            }

            formats
        }
    }

    fn set_on_connected(&mut self, callback: Option<DeviceCallback>) {
        let mut on_connected = self.on_connected.lock().unwrap();
        *on_connected = callback;
    }

    fn set_on_disconnected(&mut self, callback: Option<DeviceCallback>) {
        let mut on_disconnected = self.on_disconnected.lock().unwrap();
        *on_disconnected = callback;
    }
}

impl Drop for DeviceLinux {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
