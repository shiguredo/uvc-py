//! macOS 固有実装 (AVFoundation)

use std::ffi::{CString, c_void};
use std::sync::{Arc, Mutex, RwLock};

use crate::device::{Device, DeviceCallback};
use crate::error::UvcError;
use crate::frame::{Frame, FrameData, NativeBufferReleaseFn};
use crate::types::{DeviceInfo, Format, FormatInfo};

// CoreVideo の CVPixelBuffer 関連関数
#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    fn CVPixelBufferRetain(pixelBuffer: *mut c_void) -> *mut c_void;
    fn CVPixelBufferRelease(pixelBuffer: *mut c_void);
    fn CVPixelBufferLockBaseAddress(pixelBuffer: *mut c_void, lockFlags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(pixelBuffer: *mut c_void, lockFlags: u64) -> i32;
    fn CVPixelBufferGetWidth(pixelBuffer: *mut c_void) -> usize;
    fn CVPixelBufferGetHeight(pixelBuffer: *mut c_void) -> usize;
    fn CVPixelBufferGetPixelFormatType(pixelBuffer: *mut c_void) -> u32;
    fn CVPixelBufferGetBaseAddress(pixelBuffer: *mut c_void) -> *mut u8;
    fn CVPixelBufferGetBytesPerRow(pixelBuffer: *mut c_void) -> usize;
    fn CVPixelBufferGetBaseAddressOfPlane(pixelBuffer: *mut c_void, planeIndex: usize) -> *mut u8;
    fn CVPixelBufferGetBytesPerRowOfPlane(pixelBuffer: *mut c_void, planeIndex: usize) -> usize;
    fn CVPixelBufferIsPlanar(pixelBuffer: *mut c_void) -> bool;
}

// kCVPixelBufferLock_ReadOnly
const KCVPIXELBUFFER_LOCK_READONLY: u64 = 0x00000001;

// ピクセルフォーマット定数
const KCVPIXELFORMATTYPE_420YPCBCR8BIPLANARVIDEORANGE: u32 = 0x34323076; // '420v'
const KCVPIXELFORMATTYPE_420YPCBCR8BIPLANARFULLRANGE: u32 = 0x34323066; // '420f'
const KCVPIXELFORMATTYPE_422YPCBCR8: u32 = 0x32767579; // '2vuy' / UYVY
const KCVPIXELFORMATTYPE_422YPCBCR8_YUVS: u32 = 0x79757673; // 'yuvs' / YUY2
const KCVPIXELFORMATTYPE_32BGRA: u32 = 0x42475241; // 'BGRA'
const KCVPIXELFORMATTYPE_32ARGB: u32 = 0x00000020;
const KCVPIXELFORMATTYPE_24RGB: u32 = 0x00000018;
const KCVPIXELFORMATTYPE_24BGR: u32 = 0x32344247; // '24BG'

// objc_setAssociatedObject / objc_getAssociatedObject 用のキー
// 両方の関数で同じアドレスを使用するためにモジュールレベルで定義
static CONTEXT_KEY: u8 = 0;

/// CVPixelBuffer を retain する (frame.rs から呼び出される)
///
/// # Safety
///
/// buffer は有効な CVPixelBufferRef である必要がある
pub unsafe fn cvpixelbuffer_retain(buffer: *mut c_void) {
    unsafe { CVPixelBufferRetain(buffer) };
}

/// CVPixelBuffer を release する (frame.rs から呼び出される)
///
/// # Safety
///
/// buffer は有効な CVPixelBufferRef である必要がある
pub unsafe fn cvpixelbuffer_release(buffer: *mut c_void) {
    unsafe { CVPixelBufferRelease(buffer) };
}

/// macOS デバイス実装
pub struct DeviceMacOS {
    info: DeviceInfo,
    device_id: String,
    running: Arc<RwLock<bool>>,
    latest_frame: Arc<Mutex<Option<Frame>>>,
    on_connected: Arc<Mutex<Option<DeviceCallback>>>,
    on_disconnected: Arc<Mutex<Option<DeviceCallback>>>,
    // AVFoundation オブジェクトへの参照 (Objective-C オブジェクト)
    session: Arc<Mutex<Option<*mut c_void>>>,
    delegate: Arc<Mutex<Option<*mut c_void>>>,
}

// SAFETY: Objective-C オブジェクトはメインスレッドで適切に管理される
unsafe impl Send for DeviceMacOS {}
unsafe impl Sync for DeviceMacOS {}

impl DeviceMacOS {
    #[allow(clippy::arc_with_non_send_sync)]
    pub fn new(info: DeviceInfo, device_id: String) -> Self {
        Self {
            info,
            device_id,
            running: Arc::new(RwLock::new(false)),
            latest_frame: Arc::new(Mutex::new(None)),
            on_connected: Arc::new(Mutex::new(None)),
            on_disconnected: Arc::new(Mutex::new(None)),
            session: Arc::new(Mutex::new(None)),
            delegate: Arc::new(Mutex::new(None)),
        }
    }
}

impl Device for DeviceMacOS {
    fn start(
        &mut self,
        width: u32,
        height: u32,
        fps: u32,
        capture_format: Format,
        _output_format: Option<Format>,
    ) -> Result<(), UvcError> {
        {
            let running = self.running.read().unwrap();
            if *running {
                return Err(UvcError::AlreadyRunning);
            }
        }

        // AVFoundation セッションを開始
        start_capture_session(
            &self.device_id,
            width,
            height,
            fps,
            capture_format,
            self.latest_frame.clone(),
            self.running.clone(),
            self.session.clone(),
            self.delegate.clone(),
        )?;

        {
            let mut running = self.running.write().unwrap();
            *running = true;
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

        stop_capture_session(self.session.clone(), self.delegate.clone())?;

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
        get_device_formats(&self.device_id).unwrap_or_default()
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

impl Drop for DeviceMacOS {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

// Objective-C ランタイム関連
// ARM64 では variadic な objc_msgSend が正しく動作しないため、
// 各シグネチャごとに型付きバージョンを定義する
#[allow(clashing_extern_declarations)]
#[link(name = "objc", kind = "dylib")]
#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    fn objc_getClass(name: *const i8) -> *mut c_void;
    fn sel_registerName(name: *const i8) -> *mut c_void;

    // 引数なし
    #[link_name = "objc_msgSend"]
    fn msg_send(receiver: *mut c_void, selector: *mut c_void) -> *mut c_void;

    // 引数なし、戻り値 usize
    #[link_name = "objc_msgSend"]
    fn msg_send_usize(receiver: *mut c_void, selector: *mut c_void) -> usize;

    // *const i8 引数
    #[link_name = "objc_msgSend"]
    fn msg_send_cstr(receiver: *mut c_void, selector: *mut c_void, arg: *const i8) -> *mut c_void;

    // usize 引数
    #[link_name = "objc_msgSend"]
    fn msg_send_usize_arg(receiver: *mut c_void, selector: *mut c_void, arg: usize) -> *mut c_void;

    // u32 引数
    #[link_name = "objc_msgSend"]
    fn msg_send_u32_arg(receiver: *mut c_void, selector: *mut c_void, arg: u32) -> *mut c_void;

    // u64 引数
    #[link_name = "objc_msgSend"]
    fn msg_send_u64_arg(receiver: *mut c_void, selector: *mut c_void, arg: u64) -> *mut c_void;

    // *mut c_void 引数
    #[link_name = "objc_msgSend"]
    fn msg_send_ptr(receiver: *mut c_void, selector: *mut c_void, arg: *mut c_void) -> *mut c_void;

    // (*const *mut c_void, usize) 引数 - arrayWithObjects:count:
    #[link_name = "objc_msgSend"]
    fn msg_send_arr_count(
        receiver: *mut c_void,
        selector: *mut c_void,
        objects: *const *mut c_void,
        count: usize,
    ) -> *mut c_void;

    // (*mut c_void, *mut c_void) 引数 - setObject:forKey:
    #[link_name = "objc_msgSend"]
    fn msg_send_obj_obj(
        receiver: *mut c_void,
        selector: *mut c_void,
        obj1: *mut c_void,
        obj2: *mut c_void,
    ) -> *mut c_void;

    // (*mut c_void, *mut c_void, i64) 引数 - discoverySessionWithDeviceTypes:mediaType:position:
    #[link_name = "objc_msgSend"]
    fn msg_send_obj_obj_i64(
        receiver: *mut c_void,
        selector: *mut c_void,
        obj1: *mut c_void,
        obj2: *mut c_void,
        arg: i64,
    ) -> *mut c_void;

    // i32 引数
    #[link_name = "objc_msgSend"]
    fn msg_send_i32_arg(receiver: *mut c_void, selector: *mut c_void, arg: i32) -> *mut c_void;

    // (*mut c_void, *mut c_void) 引数、2 つ目が error pointer
    #[link_name = "objc_msgSend"]
    fn msg_send_obj_err(
        receiver: *mut c_void,
        selector: *mut c_void,
        obj: *mut c_void,
        error: *mut *mut c_void,
    ) -> *mut c_void;

    // 戻り値 f64 用
    #[link_name = "objc_msgSend"]
    fn msg_send_f64(receiver: *mut c_void, selector: *mut c_void) -> f64;
}

// CMTime 構造体 (set_frame_duration で使用)
#[repr(C)]
#[derive(Copy, Clone)]
struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

// CMTime を引数に取る msg_send (別の extern ブロックで定義)
#[allow(clashing_extern_declarations)]
#[link(name = "objc", kind = "dylib")]
unsafe extern "C" {
    #[link_name = "objc_msgSend"]
    fn msg_send_cmtime(receiver: *mut c_void, selector: *mut c_void, time: CMTime);
}

// NSObject
unsafe fn ns_alloc(class: *mut c_void) -> *mut c_void {
    unsafe {
        let sel = sel_registerName(c"alloc".as_ptr());
        msg_send(class, sel)
    }
}

unsafe fn ns_init(obj: *mut c_void) -> *mut c_void {
    unsafe {
        let sel = sel_registerName(c"init".as_ptr());
        msg_send(obj, sel)
    }
}

unsafe fn ns_release(obj: *mut c_void) {
    unsafe {
        let sel = sel_registerName(c"release".as_ptr());
        msg_send(obj, sel);
    }
}

unsafe fn ns_retain(obj: *mut c_void) -> *mut c_void {
    unsafe {
        let sel = sel_registerName(c"retain".as_ptr());
        msg_send(obj, sel)
    }
}

// NSAutoreleasePool
unsafe fn create_autorelease_pool() -> *mut c_void {
    unsafe {
        let class = objc_getClass(c"NSAutoreleasePool".as_ptr());
        let pool = ns_alloc(class);
        ns_init(pool)
    }
}

unsafe fn drain_autorelease_pool(pool: *mut c_void) {
    unsafe {
        let sel = sel_registerName(c"drain".as_ptr());
        msg_send(pool, sel);
    }
}

// NSString
unsafe fn nsstring_from_str(s: &str) -> *mut c_void {
    unsafe {
        let class = objc_getClass(c"NSString".as_ptr());
        let sel = sel_registerName(c"stringWithUTF8String:".as_ptr());
        let cstring = CString::new(s).unwrap();
        msg_send_cstr(class, sel, cstring.as_ptr())
    }
}

unsafe fn nsstring_to_string(nsstring: *mut c_void) -> String {
    if nsstring.is_null() {
        return String::new();
    }
    unsafe {
        let sel = sel_registerName(c"UTF8String".as_ptr());
        let cstr = msg_send(nsstring, sel) as *const i8;
        if cstr.is_null() {
            return String::new();
        }
        std::ffi::CStr::from_ptr(cstr)
            .to_string_lossy()
            .into_owned()
    }
}

// NSArray
unsafe fn nsarray_count(array: *mut c_void) -> usize {
    unsafe {
        let sel = sel_registerName(c"count".as_ptr());
        msg_send_usize(array, sel)
    }
}

unsafe fn nsarray_object_at_index(array: *mut c_void, index: usize) -> *mut c_void {
    unsafe {
        let sel = sel_registerName(c"objectAtIndex:".as_ptr());
        msg_send_usize_arg(array, sel, index)
    }
}

/// デバイス列挙
pub fn list_devices() -> Result<Vec<DeviceInfo>, UvcError> {
    unsafe {
        // autorelease pool を作成
        let pool = create_autorelease_pool();

        // AVCaptureDeviceDiscoverySession を使用してデバイスを列挙
        let discovery_class = objc_getClass(c"AVCaptureDeviceDiscoverySession".as_ptr());
        if discovery_class.is_null() {
            drain_autorelease_pool(pool);
            return Err(UvcError::PlatformError(
                "AVCaptureDeviceDiscoverySession クラスが見つかりません".to_string(),
            ));
        }

        // デバイスタイプの配列を作成
        let nsarray_class = objc_getClass(c"NSArray".as_ptr());
        let built_in_type = nsstring_from_str("AVCaptureDeviceTypeBuiltInWideAngleCamera");
        let external_type = nsstring_from_str("AVCaptureDeviceTypeExternal");

        let array_with_objects_sel = sel_registerName(c"arrayWithObjects:count:".as_ptr());
        let device_types: [*mut c_void; 2] = [built_in_type, external_type];
        let device_types_array = msg_send_arr_count(
            nsarray_class,
            array_with_objects_sel,
            device_types.as_ptr(),
            2usize,
        );

        // メディアタイプ
        let video_media_type = nsstring_from_str("vide");

        // ディスカバリセッションを作成
        let discovery_sel =
            sel_registerName(c"discoverySessionWithDeviceTypes:mediaType:position:".as_ptr());
        let discovery_session = msg_send_obj_obj_i64(
            discovery_class,
            discovery_sel,
            device_types_array,
            video_media_type,
            0i64, // AVCaptureDevicePositionUnspecified (NSInteger = i64 on 64-bit)
        );

        if discovery_session.is_null() {
            drain_autorelease_pool(pool);
            return Err(UvcError::PlatformError(
                "ディスカバリセッションの作成に失敗しました".to_string(),
            ));
        }

        // デバイスリストを取得
        let devices_sel = sel_registerName(c"devices".as_ptr());
        let devices = msg_send(discovery_session, devices_sel);

        let count = nsarray_count(devices);
        let mut result = Vec::with_capacity(count);

        for i in 0..count {
            let device = nsarray_object_at_index(devices, i);

            // localizedName を取得
            let localized_name_sel = sel_registerName(c"localizedName".as_ptr());
            let name_nsstring = msg_send(device, localized_name_sel);
            let name = nsstring_to_string(name_nsstring);

            // uniqueID を取得
            let unique_id_sel = sel_registerName(c"uniqueID".as_ptr());
            let unique_id_nsstring = msg_send(device, unique_id_sel);
            let unique_id = nsstring_to_string(unique_id_nsstring);

            result.push(DeviceInfo::new(name, unique_id, i as u32));
        }

        drain_autorelease_pool(pool);
        Ok(result)
    }
}

/// デバイスをオープン
pub fn open_device(index: u32) -> Result<Box<dyn Device>, UvcError> {
    let devices = list_devices()?;

    if index as usize >= devices.len() {
        return Err(UvcError::DeviceNotFound(index));
    }

    let info = devices[index as usize].clone();
    let device_id = info.unique_id.clone();

    Ok(Box::new(DeviceMacOS::new(info, device_id)))
}

/// デバイスのサポートフォーマットを取得
fn get_device_formats(device_id: &str) -> Result<Vec<FormatInfo>, UvcError> {
    unsafe {
        // AVCaptureDevice を取得
        let device = get_av_capture_device(device_id)?;

        // formats を取得
        let formats_sel = sel_registerName(c"formats".as_ptr());
        let formats = msg_send(device, formats_sel);

        let count = nsarray_count(formats);
        let mut result = Vec::new();

        for i in 0..count {
            let format = nsarray_object_at_index(formats, i);

            // formatDescription を取得
            let format_desc_sel = sel_registerName(c"formatDescription".as_ptr());
            let format_desc = msg_send(format, format_desc_sel);

            // 解像度を取得
            let (width, height) = get_format_dimensions(format_desc);

            // ピクセルフォーマットを取得
            let pixel_format = get_format_media_subtype(format_desc);
            let uvc_format = match pixel_format {
                KCVPIXELFORMATTYPE_420YPCBCR8BIPLANARVIDEORANGE
                | KCVPIXELFORMATTYPE_420YPCBCR8BIPLANARFULLRANGE => Some(Format::NV12),
                KCVPIXELFORMATTYPE_422YPCBCR8 | KCVPIXELFORMATTYPE_422YPCBCR8_YUVS => {
                    Some(Format::YUY2)
                }
                KCVPIXELFORMATTYPE_32BGRA | KCVPIXELFORMATTYPE_32ARGB => Some(Format::RGBA),
                KCVPIXELFORMATTYPE_24RGB | KCVPIXELFORMATTYPE_24BGR => Some(Format::RGB),
                _ => None,
            };

            if let Some(fmt) = uvc_format {
                // フレームレート範囲を取得
                let frame_rate_ranges_sel =
                    sel_registerName(c"videoSupportedFrameRateRanges".as_ptr());
                let frame_rate_ranges = msg_send(format, frame_rate_ranges_sel);

                let ranges_count = nsarray_count(frame_rate_ranges);
                for j in 0..ranges_count {
                    let range = nsarray_object_at_index(frame_rate_ranges, j);
                    let max_fps = get_frame_rate_range_max(range);

                    result.push(FormatInfo::new(width, height, max_fps as u32, fmt));
                }
            }
        }

        Ok(result)
    }
}

/// AVCaptureDevice を uniqueID から取得
unsafe fn get_av_capture_device(device_id: &str) -> Result<*mut c_void, UvcError> {
    unsafe {
        let device_class = objc_getClass(c"AVCaptureDevice".as_ptr());
        let device_with_id_sel = sel_registerName(c"deviceWithUniqueID:".as_ptr());
        let device_id_nsstring = nsstring_from_str(device_id);
        let device = msg_send_ptr(device_class, device_with_id_sel, device_id_nsstring);

        if device.is_null() {
            return Err(UvcError::PlatformError(format!(
                "デバイスが見つかりません: {}",
                device_id
            )));
        }

        Ok(device)
    }
}

/// CMFormatDescription から解像度を取得
unsafe fn get_format_dimensions(format_desc: *mut c_void) -> (u32, u32) {
    #[link(name = "CoreMedia", kind = "framework")]
    unsafe extern "C" {
        fn CMVideoFormatDescriptionGetDimensions(videoDesc: *mut c_void) -> CMVideoDimensions;
    }

    #[repr(C)]
    struct CMVideoDimensions {
        width: i32,
        height: i32,
    }

    unsafe {
        let dims = CMVideoFormatDescriptionGetDimensions(format_desc);
        (dims.width as u32, dims.height as u32)
    }
}

/// CMFormatDescription からメディアサブタイプを取得
unsafe fn get_format_media_subtype(format_desc: *mut c_void) -> u32 {
    #[link(name = "CoreMedia", kind = "framework")]
    unsafe extern "C" {
        fn CMFormatDescriptionGetMediaSubType(desc: *mut c_void) -> u32;
    }

    unsafe { CMFormatDescriptionGetMediaSubType(format_desc) }
}

/// AVFrameRateRange から最大フレームレートを取得
unsafe fn get_frame_rate_range_max(range: *mut c_void) -> f64 {
    unsafe {
        let sel = sel_registerName(c"maxFrameRate".as_ptr());

        #[cfg(target_arch = "x86_64")]
        {
            #[link(name = "objc", kind = "dylib")]
            unsafe extern "C" {
                fn objc_msgSend_fpret(receiver: *mut c_void, selector: *mut c_void) -> f64;
            }
            objc_msgSend_fpret(range, sel)
        }

        #[cfg(target_arch = "aarch64")]
        {
            // ARM64 では通常の objc_msgSend で OK
            msg_send_f64(range, sel)
        }
    }
}

/// キャプチャセッションを開始
#[allow(clippy::too_many_arguments)]
fn start_capture_session(
    device_id: &str,
    width: u32,
    height: u32,
    fps: u32,
    capture_format: Format,
    latest_frame: Arc<Mutex<Option<Frame>>>,
    _running: Arc<RwLock<bool>>,
    session_holder: Arc<Mutex<Option<*mut c_void>>>,
    delegate_holder: Arc<Mutex<Option<*mut c_void>>>,
) -> Result<(), UvcError> {
    unsafe {
        // AVCaptureDevice を取得
        let device = get_av_capture_device(device_id)?;

        // AVCaptureSession を作成
        let session_class = objc_getClass(c"AVCaptureSession".as_ptr());
        let session = ns_alloc(session_class);
        let session = ns_init(session);

        // beginConfiguration
        let begin_config_sel = sel_registerName(c"beginConfiguration".as_ptr());
        msg_send(session, begin_config_sel);

        // デバイスをロック
        let lock_sel = sel_registerName(c"lockForConfiguration:".as_ptr());
        let mut error: *mut c_void = std::ptr::null_mut();
        let lock_result = msg_send_obj_err(device, lock_sel, std::ptr::null_mut(), &mut error);
        if lock_result.is_null() {
            let commit_sel = sel_registerName(c"commitConfiguration".as_ptr());
            msg_send(session, commit_sel);
            ns_release(session);
            return Err(UvcError::PlatformError(
                "デバイスのロックに失敗しました".to_string(),
            ));
        }

        // 最適なフォーマットを探す
        let best_format = find_best_format(device, width, height, fps, capture_format)?;

        // フォーマットを設定
        let set_active_format_sel = sel_registerName(c"setActiveFormat:".as_ptr());
        msg_send_ptr(device, set_active_format_sel, best_format);

        // フレームレートを設定
        set_frame_duration(device, fps);

        // デバイスをアンロック
        let unlock_sel = sel_registerName(c"unlockForConfiguration".as_ptr());
        msg_send(device, unlock_sel);

        // AVCaptureDeviceInput を作成
        let input_class = objc_getClass(c"AVCaptureDeviceInput".as_ptr());
        let input_with_device_sel = sel_registerName(c"deviceInputWithDevice:error:".as_ptr());
        let input = msg_send_obj_err(input_class, input_with_device_sel, device, &mut error);

        if input.is_null() {
            let commit_sel = sel_registerName(c"commitConfiguration".as_ptr());
            msg_send(session, commit_sel);
            ns_release(session);
            return Err(UvcError::PlatformError(
                "入力デバイスの作成に失敗しました".to_string(),
            ));
        }

        // 入力を追加
        let can_add_input_sel = sel_registerName(c"canAddInput:".as_ptr());
        let can_add = msg_send_ptr(session, can_add_input_sel, input) as usize;
        if can_add != 0 {
            let add_input_sel = sel_registerName(c"addInput:".as_ptr());
            msg_send_ptr(session, add_input_sel, input);
        }

        // AVCaptureVideoDataOutput を作成
        let output_class = objc_getClass(c"AVCaptureVideoDataOutput".as_ptr());
        let output = ns_alloc(output_class);
        let output = ns_init(output);

        // alwaysDiscardsLateVideoFrames = YES
        let set_discards_sel = sel_registerName(c"setAlwaysDiscardsLateVideoFrames:".as_ptr());
        msg_send_i32_arg(output, set_discards_sel, 1i32);

        // ビデオ設定
        let pixel_format = match capture_format {
            Format::NV12 => KCVPIXELFORMATTYPE_420YPCBCR8BIPLANARFULLRANGE,
            Format::YUY2 => KCVPIXELFORMATTYPE_422YPCBCR8_YUVS,
            Format::RGBA | Format::RGB | Format::MJPEG => KCVPIXELFORMATTYPE_32BGRA,
        };

        let video_settings = create_video_settings(pixel_format, width, height);
        let set_video_settings_sel = sel_registerName(c"setVideoSettings:".as_ptr());
        msg_send_ptr(output, set_video_settings_sel, video_settings);

        // デリゲートとキューを設定
        let delegate = create_sample_buffer_delegate(latest_frame.clone());
        let queue = create_dispatch_queue();
        let set_delegate_sel = sel_registerName(c"setSampleBufferDelegate:queue:".as_ptr());
        msg_send_obj_obj(output, set_delegate_sel, delegate, queue);

        // 出力を追加
        let can_add_output_sel = sel_registerName(c"canAddOutput:".as_ptr());
        let can_add = msg_send_ptr(session, can_add_output_sel, output) as usize;
        if can_add != 0 {
            let add_output_sel = sel_registerName(c"addOutput:".as_ptr());
            msg_send_ptr(session, add_output_sel, output);
        }

        // commitConfiguration
        let commit_sel = sel_registerName(c"commitConfiguration".as_ptr());
        msg_send(session, commit_sel);

        // startRunning
        let start_running_sel = sel_registerName(c"startRunning".as_ptr());
        msg_send(session, start_running_sel);

        // セッションとデリゲートを保持
        {
            let mut session_guard = session_holder.lock().unwrap();
            *session_guard = Some(ns_retain(session));
        }
        {
            let mut delegate_guard = delegate_holder.lock().unwrap();
            *delegate_guard = Some(ns_retain(delegate));
        }

        Ok(())
    }
}

/// 最適なフォーマットを探す
unsafe fn find_best_format(
    device: *mut c_void,
    width: u32,
    height: u32,
    fps: u32,
    capture_format: Format,
) -> Result<*mut c_void, UvcError> {
    unsafe {
        let formats_sel = sel_registerName(c"formats".as_ptr());
        let formats = msg_send(device, formats_sel);
        let count = nsarray_count(formats);

        let mut best_format: *mut c_void = std::ptr::null_mut();
        let mut best_range_width = f64::MAX;

        for i in 0..count {
            let format = nsarray_object_at_index(formats, i);

            let format_desc_sel = sel_registerName(c"formatDescription".as_ptr());
            let format_desc = msg_send(format, format_desc_sel);

            let (fmt_width, fmt_height) = get_format_dimensions(format_desc);
            if fmt_width != width || fmt_height != height {
                continue;
            }

            let pixel_format = get_format_media_subtype(format_desc);
            let format_match = match capture_format {
                Format::NV12 => {
                    pixel_format == KCVPIXELFORMATTYPE_420YPCBCR8BIPLANARVIDEORANGE
                        || pixel_format == KCVPIXELFORMATTYPE_420YPCBCR8BIPLANARFULLRANGE
                }
                Format::YUY2 => {
                    pixel_format == KCVPIXELFORMATTYPE_422YPCBCR8
                        || pixel_format == KCVPIXELFORMATTYPE_422YPCBCR8_YUVS
                }
                Format::RGBA => {
                    pixel_format == KCVPIXELFORMATTYPE_32BGRA
                        || pixel_format == KCVPIXELFORMATTYPE_32ARGB
                }
                Format::RGB => {
                    pixel_format == KCVPIXELFORMATTYPE_24RGB
                        || pixel_format == KCVPIXELFORMATTYPE_24BGR
                }
                Format::MJPEG => false, // MJPEG はサポートしない
            };

            if !format_match {
                continue;
            }

            // フレームレート範囲をチェック
            let frame_rate_ranges_sel = sel_registerName(c"videoSupportedFrameRateRanges".as_ptr());
            let frame_rate_ranges = msg_send(format, frame_rate_ranges_sel);
            let ranges_count = nsarray_count(frame_rate_ranges);

            for j in 0..ranges_count {
                let range = nsarray_object_at_index(frame_rate_ranges, j);
                let min_fps = get_frame_rate_range_min(range);
                let max_fps = get_frame_rate_range_max(range);

                // 指定 fps が範囲内にあるか (0.5 の許容誤差)
                if (fps as f64) >= min_fps - 0.5 && (fps as f64) <= max_fps + 0.5 {
                    let range_width = max_fps - min_fps;
                    if range_width < best_range_width {
                        best_format = format;
                        best_range_width = range_width;
                    }
                }
            }
        }

        if best_format.is_null() {
            let fmt_str = match capture_format {
                Format::MJPEG => "MJPEG (not supported)",
                Format::YUY2 => "YUY2",
                Format::NV12 => "NV12",
                Format::RGB => "RGB",
                Format::RGBA => "RGBA",
            };
            return Err(UvcError::FormatError(format!(
                "サポートされていないフォーマット: {}x{}@{} {}",
                width, height, fps, fmt_str
            )));
        }

        Ok(best_format)
    }
}

/// AVFrameRateRange から最小フレームレートを取得
unsafe fn get_frame_rate_range_min(range: *mut c_void) -> f64 {
    unsafe {
        let sel = sel_registerName(c"minFrameRate".as_ptr());

        #[cfg(target_arch = "x86_64")]
        {
            #[link(name = "objc", kind = "dylib")]
            unsafe extern "C" {
                fn objc_msgSend_fpret(receiver: *mut c_void, selector: *mut c_void) -> f64;
            }
            objc_msgSend_fpret(range, sel)
        }

        #[cfg(target_arch = "aarch64")]
        {
            msg_send_f64(range, sel)
        }
    }
}

/// フレーム期間を設定
unsafe fn set_frame_duration(device: *mut c_void, fps: u32) {
    let frame_duration = CMTime {
        value: 1,
        timescale: fps as i32,
        flags: 1, // kCMTimeFlags_Valid
        epoch: 0,
    };

    unsafe {
        let set_min_sel = sel_registerName(c"setActiveVideoMinFrameDuration:".as_ptr());
        let set_max_sel = sel_registerName(c"setActiveVideoMaxFrameDuration:".as_ptr());

        msg_send_cmtime(device, set_min_sel, frame_duration);
        msg_send_cmtime(device, set_max_sel, frame_duration);
    }
}

/// ビデオ設定の NSDictionary を作成
unsafe fn create_video_settings(pixel_format: u32, width: u32, height: u32) -> *mut c_void {
    unsafe {
        let dict_class = objc_getClass(c"NSMutableDictionary".as_ptr());
        let dict = ns_alloc(dict_class);
        let dict = ns_init(dict);

        let set_object_sel = sel_registerName(c"setObject:forKey:".as_ptr());

        // kCVPixelBufferPixelFormatTypeKey
        let pixel_format_key = nsstring_from_str("PixelFormatType");
        let number_class = objc_getClass(c"NSNumber".as_ptr());
        let number_with_uint_sel = sel_registerName(c"numberWithUnsignedInt:".as_ptr());
        let pixel_format_value = msg_send_u32_arg(number_class, number_with_uint_sel, pixel_format);
        msg_send_obj_obj(dict, set_object_sel, pixel_format_value, pixel_format_key);

        // kCVPixelBufferWidthKey
        let width_key = nsstring_from_str("Width");
        let width_value = msg_send_u32_arg(number_class, number_with_uint_sel, width);
        msg_send_obj_obj(dict, set_object_sel, width_value, width_key);

        // kCVPixelBufferHeightKey
        let height_key = nsstring_from_str("Height");
        let height_value = msg_send_u32_arg(number_class, number_with_uint_sel, height);
        msg_send_obj_obj(dict, set_object_sel, height_value, height_key);

        dict
    }
}

/// dispatch_queue を作成
unsafe fn create_dispatch_queue() -> *mut c_void {
    #[link(name = "System", kind = "dylib")]
    unsafe extern "C" {
        fn dispatch_queue_create(label: *const i8, attr: *mut c_void) -> *mut c_void;
    }

    unsafe { dispatch_queue_create(c"uvc.capture".as_ptr(), std::ptr::null_mut()) }
}

// サンプルバッファデリゲート用のコールバック関数とコンテキスト
struct DelegateContext {
    latest_frame: Arc<Mutex<Option<Frame>>>,
}

/// Send/Sync 可能なポインタラッパー
#[derive(Clone, Copy)]
struct SendPtr(*mut c_void);

// SAFETY: Objective-C クラスポインタは一度登録されると不変であり、
// 複数スレッドから安全に読み取り可能
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

// デリゲートクラスを動的に作成するためのグローバル変数
static DELEGATE_CLASS_REGISTERED: std::sync::Once = std::sync::Once::new();
static DELEGATE_CLASS: Mutex<SendPtr> = Mutex::new(SendPtr(std::ptr::null_mut()));

/// サンプルバッファデリゲートを作成
unsafe fn create_sample_buffer_delegate(latest_frame: Arc<Mutex<Option<Frame>>>) -> *mut c_void {
    // デリゲートクラスを登録
    DELEGATE_CLASS_REGISTERED.call_once(|| {
        unsafe { register_delegate_class() };
    });

    unsafe {
        let delegate_class = DELEGATE_CLASS.lock().unwrap().0;
        let delegate = ns_alloc(delegate_class);
        let delegate = ns_init(delegate);

        // コンテキストを関連付け
        let context = Box::new(DelegateContext { latest_frame });
        let context_ptr = Box::into_raw(context);

        // objc_setAssociatedObject でコンテキストを保持
        #[link(name = "objc", kind = "dylib")]
        unsafe extern "C" {
            fn objc_setAssociatedObject(
                object: *mut c_void,
                key: *const c_void,
                value: *mut c_void,
                policy: usize,
            );
        }

        // NSNumber でポインタをラップ
        let number_class = objc_getClass(c"NSNumber".as_ptr());
        let number_with_ptr_sel = sel_registerName(c"numberWithUnsignedLongLong:".as_ptr());
        let context_number =
            msg_send_u64_arg(number_class, number_with_ptr_sel, context_ptr as u64);

        // OBJC_ASSOCIATION_RETAIN_NONATOMIC = 1
        objc_setAssociatedObject(
            delegate,
            &CONTEXT_KEY as *const _ as *const c_void,
            context_number,
            1,
        );

        delegate
    }
}

/// デリゲートクラスを登録
unsafe fn register_delegate_class() {
    #[link(name = "objc", kind = "dylib")]
    unsafe extern "C" {
        fn objc_allocateClassPair(
            superclass: *mut c_void,
            name: *const i8,
            extraBytes: usize,
        ) -> *mut c_void;
        fn objc_registerClassPair(cls: *mut c_void);
        fn class_addMethod(
            cls: *mut c_void,
            name: *mut c_void,
            imp: *mut c_void,
            types: *const i8,
        ) -> bool;
        fn class_addProtocol(cls: *mut c_void, protocol: *mut c_void) -> bool;
        fn objc_getProtocol(name: *const i8) -> *mut c_void;
    }

    unsafe {
        let nsobject_class = objc_getClass(c"NSObject".as_ptr());
        let delegate_class =
            objc_allocateClassPair(nsobject_class, c"UVCCaptureDelegateRust".as_ptr(), 0);

        // AVCaptureVideoDataOutputSampleBufferDelegate プロトコルを追加
        let protocol = objc_getProtocol(c"AVCaptureVideoDataOutputSampleBufferDelegate".as_ptr());
        if !protocol.is_null() {
            class_addProtocol(delegate_class, protocol);
        }

        // captureOutput:didOutputSampleBuffer:fromConnection: メソッドを追加
        let sel = sel_registerName(c"captureOutput:didOutputSampleBuffer:fromConnection:".as_ptr());
        class_addMethod(
            delegate_class,
            sel,
            capture_output_did_output_sample_buffer as *mut c_void,
            c"v@:@@@".as_ptr(),
        );

        // dealloc メソッドを追加
        let dealloc_sel = sel_registerName(c"dealloc".as_ptr());
        class_addMethod(
            delegate_class,
            dealloc_sel,
            delegate_dealloc as *mut c_void,
            c"v@:".as_ptr(),
        );

        objc_registerClassPair(delegate_class);

        *DELEGATE_CLASS.lock().unwrap() = SendPtr(delegate_class);
    }
}

/// キャプチャ出力コールバック
extern "C" fn capture_output_did_output_sample_buffer(
    this: *mut c_void,
    _sel: *mut c_void,
    _output: *mut c_void,
    sample_buffer: *mut c_void,
    _connection: *mut c_void,
) {
    unsafe {
        // コンテキストを取得
        #[link(name = "objc", kind = "dylib")]
        unsafe extern "C" {
            fn objc_getAssociatedObject(object: *mut c_void, key: *const c_void) -> *mut c_void;
        }

        let context_number =
            objc_getAssociatedObject(this, &CONTEXT_KEY as *const _ as *const c_void);
        if context_number.is_null() {
            return;
        }

        let unsigned_long_long_value_sel = sel_registerName(c"unsignedLongLongValue".as_ptr());
        let context_ptr =
            msg_send(context_number, unsigned_long_long_value_sel) as *mut DelegateContext;
        if context_ptr.is_null() {
            return;
        }

        let context = &*context_ptr;

        // CMSampleBuffer から CVImageBuffer を取得
        #[link(name = "CoreMedia", kind = "framework")]
        unsafe extern "C" {
            fn CMSampleBufferGetImageBuffer(sbuf: *mut c_void) -> *mut c_void;
            fn CMSampleBufferGetPresentationTimeStamp(sbuf: *mut c_void) -> CMTime;
            fn CMTimeGetSeconds(time: CMTime) -> f64;
        }

        #[repr(C)]
        #[derive(Copy, Clone)]
        struct CMTime {
            value: i64,
            timescale: i32,
            flags: u32,
            epoch: i64,
        }

        let image_buffer = CMSampleBufferGetImageBuffer(sample_buffer);
        if image_buffer.is_null() {
            return;
        }

        // ピクセルバッファをロック
        let lock_result = CVPixelBufferLockBaseAddress(image_buffer, KCVPIXELBUFFER_LOCK_READONLY);
        if lock_result != 0 {
            return;
        }

        let width = CVPixelBufferGetWidth(image_buffer) as u32;
        let height = CVPixelBufferGetHeight(image_buffer) as u32;
        let pixel_format = CVPixelBufferGetPixelFormatType(image_buffer);

        // フォーマットを判定
        let format = match pixel_format {
            KCVPIXELFORMATTYPE_420YPCBCR8BIPLANARVIDEORANGE
            | KCVPIXELFORMATTYPE_420YPCBCR8BIPLANARFULLRANGE => Format::NV12,
            KCVPIXELFORMATTYPE_422YPCBCR8 | KCVPIXELFORMATTYPE_422YPCBCR8_YUVS => Format::YUY2,
            KCVPIXELFORMATTYPE_32BGRA | KCVPIXELFORMATTYPE_32ARGB => Format::RGBA,
            _ => {
                CVPixelBufferUnlockBaseAddress(image_buffer, KCVPIXELBUFFER_LOCK_READONLY);
                return;
            }
        };

        // CVPixelBuffer を retain
        CVPixelBufferRetain(image_buffer);

        // リリース関数
        // ポインタ値を usize としてキャプチャして Send + Sync にする
        let buffer_addr = image_buffer as usize;
        let release_fn: NativeBufferReleaseFn = Box::new(move || {
            let buffer = buffer_addr as *mut c_void;
            CVPixelBufferUnlockBaseAddress(buffer, KCVPIXELBUFFER_LOCK_READONLY);
            CVPixelBufferRelease(buffer);
        });

        // タイムスタンプを取得
        let pts = CMSampleBufferGetPresentationTimeStamp(sample_buffer);
        let timestamp = (CMTimeGetSeconds(pts) * 1_000_000.0) as u64;

        // フレームデータを作成
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
            native_buffer: Some(image_buffer),
            native_buffer_release: Some(release_fn),
        };

        // プレーンデータを設定
        let is_planar = CVPixelBufferIsPlanar(image_buffer);
        if is_planar {
            // NV12 など
            frame_data.y_plane = Some(CVPixelBufferGetBaseAddressOfPlane(image_buffer, 0));
            frame_data.y_stride = CVPixelBufferGetBytesPerRowOfPlane(image_buffer, 0);
            frame_data.uv_plane = Some(CVPixelBufferGetBaseAddressOfPlane(image_buffer, 1));
            frame_data.uv_stride = CVPixelBufferGetBytesPerRowOfPlane(image_buffer, 1);
        } else {
            // YUY2, RGBA など
            frame_data.packed_plane = Some(CVPixelBufferGetBaseAddress(image_buffer));
            frame_data.packed_stride = CVPixelBufferGetBytesPerRow(image_buffer);
        }

        let frame = Frame::new(frame_data);

        // 最新フレームを更新
        if let Ok(mut guard) = context.latest_frame.lock() {
            *guard = Some(frame);
        }
    }
}

/// デリゲートの dealloc
extern "C" fn delegate_dealloc(this: *mut c_void, _sel: *mut c_void) {
    unsafe {
        // コンテキストを解放
        #[link(name = "objc", kind = "dylib")]
        unsafe extern "C" {
            fn objc_getAssociatedObject(object: *mut c_void, key: *const c_void) -> *mut c_void;
        }

        let context_number =
            objc_getAssociatedObject(this, &CONTEXT_KEY as *const _ as *const c_void);
        if !context_number.is_null() {
            let unsigned_long_long_value_sel = sel_registerName(c"unsignedLongLongValue".as_ptr());
            let context_ptr =
                msg_send(context_number, unsigned_long_long_value_sel) as *mut DelegateContext;
            if !context_ptr.is_null() {
                let _ = Box::from_raw(context_ptr);
            }
        }

        // スーパークラスの dealloc を呼ぶ
        let super_class = objc_getClass(c"NSObject".as_ptr());
        let dealloc_sel = sel_registerName(c"dealloc".as_ptr());

        #[repr(C)]
        struct ObjcSuper {
            receiver: *mut c_void,
            super_class: *mut c_void,
        }

        unsafe extern "C" {
            fn objc_msgSendSuper(super_: *mut ObjcSuper, sel: *mut c_void, ...);
        }

        let mut super_struct = ObjcSuper {
            receiver: this,
            super_class,
        };
        objc_msgSendSuper(&mut super_struct as *mut _, dealloc_sel);
    }
}

/// キャプチャセッションを停止
fn stop_capture_session(
    session_holder: Arc<Mutex<Option<*mut c_void>>>,
    delegate_holder: Arc<Mutex<Option<*mut c_void>>>,
) -> Result<(), UvcError> {
    unsafe {
        // セッションを停止
        if let Some(session) = session_holder.lock().unwrap().take() {
            let stop_running_sel = sel_registerName(c"stopRunning".as_ptr());
            msg_send(session, stop_running_sel);
            ns_release(session);
        }

        // デリゲートを解放
        if let Some(delegate) = delegate_holder.lock().unwrap().take() {
            ns_release(delegate);
        }

        Ok(())
    }
}
