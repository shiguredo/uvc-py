//! macOS 固有実装 (AVFoundation)
//!
//! このモジュールは AVFoundation を使用して macOS 上でカメラキャプチャを行う。
//! Objective-C ランタイムとの FFI を直接使用している。

use std::ffi::{CString, c_void};
use std::sync::{Arc, Mutex, RwLock};

use crate::device::{Device, DeviceCallback};
use crate::error::UvcError;
use crate::frame::{Frame, FrameData, NativeBufferReleaseFn};
use crate::types::{DeviceInfo, Format, FormatInfo};

// ============================================================================
// FFI 宣言: Objective-C ランタイム
// ============================================================================

#[link(name = "objc", kind = "dylib")]
unsafe extern "C" {
    /// クラス名から Objective-C クラスを取得
    fn objc_getClass(name: *const i8) -> *mut c_void;

    /// セレクタ名からセレクタを登録/取得
    fn sel_registerName(name: *const i8) -> *mut c_void;

    /// オブジェクトに関連付けられた値を設定
    fn objc_setAssociatedObject(
        object: *mut c_void,
        key: *const c_void,
        value: *mut c_void,
        policy: usize,
    );

    /// オブジェクトに関連付けられた値を取得
    fn objc_getAssociatedObject(object: *mut c_void, key: *const c_void) -> *mut c_void;

    /// 新しい Objective-C クラスを割り当て
    fn objc_allocateClassPair(
        superclass: *mut c_void,
        name: *const i8,
        extra_bytes: usize,
    ) -> *mut c_void;

    /// クラスを登録
    fn objc_registerClassPair(cls: *mut c_void);

    /// クラスにメソッドを追加
    fn class_addMethod(
        cls: *mut c_void,
        name: *mut c_void,
        imp: *mut c_void,
        types: *const i8,
    ) -> bool;

    /// クラスにプロトコルを追加
    fn class_addProtocol(cls: *mut c_void, protocol: *mut c_void) -> bool;

    /// プロトコル名からプロトコルを取得
    fn objc_getProtocol(name: *const i8) -> *mut c_void;
}

// ============================================================================
// FFI 宣言: objc_msgSend バリアント
//
// ARM64 では variadic な objc_msgSend が正しく動作しないため、
// 各シグネチャごとに型付きバージョンを定義する
// ============================================================================

#[allow(clashing_extern_declarations)]
#[link(name = "objc", kind = "dylib")]
unsafe extern "C" {
    /// 引数なし、戻り値 *mut c_void
    #[link_name = "objc_msgSend"]
    fn msg_send(receiver: *mut c_void, selector: *mut c_void) -> *mut c_void;

    /// 引数なし、戻り値 usize
    #[link_name = "objc_msgSend"]
    fn msg_send_usize(receiver: *mut c_void, selector: *mut c_void) -> usize;

    /// 引数なし、戻り値 f64
    #[link_name = "objc_msgSend"]
    fn msg_send_f64(receiver: *mut c_void, selector: *mut c_void) -> f64;

    /// 引数: *const i8
    #[link_name = "objc_msgSend"]
    fn msg_send_cstr(receiver: *mut c_void, selector: *mut c_void, arg: *const i8) -> *mut c_void;

    /// 引数: usize
    #[link_name = "objc_msgSend"]
    fn msg_send_usize_arg(receiver: *mut c_void, selector: *mut c_void, arg: usize) -> *mut c_void;

    /// 引数: u32
    #[link_name = "objc_msgSend"]
    fn msg_send_u32_arg(receiver: *mut c_void, selector: *mut c_void, arg: u32) -> *mut c_void;

    /// 引数: u64
    #[link_name = "objc_msgSend"]
    fn msg_send_u64_arg(receiver: *mut c_void, selector: *mut c_void, arg: u64) -> *mut c_void;

    /// 引数: i32
    #[link_name = "objc_msgSend"]
    fn msg_send_i32_arg(receiver: *mut c_void, selector: *mut c_void, arg: i32) -> *mut c_void;

    /// 引数: *mut c_void
    #[link_name = "objc_msgSend"]
    fn msg_send_ptr(receiver: *mut c_void, selector: *mut c_void, arg: *mut c_void) -> *mut c_void;

    /// 引数: (*const *mut c_void, usize) - arrayWithObjects:count: 用
    #[link_name = "objc_msgSend"]
    fn msg_send_arr_count(
        receiver: *mut c_void,
        selector: *mut c_void,
        objects: *const *mut c_void,
        count: usize,
    ) -> *mut c_void;

    /// 引数: (*mut c_void, *mut c_void) - setObject:forKey: 用
    #[link_name = "objc_msgSend"]
    fn msg_send_obj_obj(
        receiver: *mut c_void,
        selector: *mut c_void,
        obj1: *mut c_void,
        obj2: *mut c_void,
    ) -> *mut c_void;

    /// 引数: (*mut c_void, *mut c_void, i64) - discoverySessionWithDeviceTypes:mediaType:position: 用
    #[link_name = "objc_msgSend"]
    fn msg_send_obj_obj_i64(
        receiver: *mut c_void,
        selector: *mut c_void,
        obj1: *mut c_void,
        obj2: *mut c_void,
        arg: i64,
    ) -> *mut c_void;

    /// 引数: (*mut c_void, *mut *mut c_void) - error ポインタ付き
    #[link_name = "objc_msgSend"]
    fn msg_send_obj_err(
        receiver: *mut c_void,
        selector: *mut c_void,
        obj: *mut c_void,
        error: *mut *mut c_void,
    ) -> *mut c_void;

    /// 引数: CMTime
    #[link_name = "objc_msgSend"]
    fn msg_send_cmtime(receiver: *mut c_void, selector: *mut c_void, time: CMTime);
}

// objc_msgSendSuper - スーパークラスのメソッドを呼び出す
#[link(name = "objc", kind = "dylib")]
unsafe extern "C" {
    fn objc_msgSendSuper(super_: *mut ObjcSuper, sel: *mut c_void, ...);
}

/// x86_64 では浮動小数点の戻り値に objc_msgSend_fpret を使用する
#[cfg(target_arch = "x86_64")]
#[link(name = "objc", kind = "dylib")]
unsafe extern "C" {
    fn objc_msgSend_fpret(receiver: *mut c_void, selector: *mut c_void) -> f64;
}

// ============================================================================
// FFI 宣言: CoreVideo
// ============================================================================

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    fn CVPixelBufferRetain(pixel_buffer: *mut c_void) -> *mut c_void;
    fn CVPixelBufferRelease(pixel_buffer: *mut c_void);
    fn CVPixelBufferLockBaseAddress(pixel_buffer: *mut c_void, lock_flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(pixel_buffer: *mut c_void, lock_flags: u64) -> i32;
    fn CVPixelBufferGetWidth(pixel_buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetHeight(pixel_buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetPixelFormatType(pixel_buffer: *mut c_void) -> u32;
    fn CVPixelBufferGetBaseAddress(pixel_buffer: *mut c_void) -> *mut u8;
    fn CVPixelBufferGetBytesPerRow(pixel_buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetBaseAddressOfPlane(pixel_buffer: *mut c_void, plane_index: usize)
    -> *mut u8;
    fn CVPixelBufferGetBytesPerRowOfPlane(pixel_buffer: *mut c_void, plane_index: usize) -> usize;
    fn CVPixelBufferIsPlanar(pixel_buffer: *mut c_void) -> bool;
}

// ============================================================================
// FFI 宣言: CoreMedia
// ============================================================================

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    fn CMVideoFormatDescriptionGetDimensions(video_desc: *mut c_void) -> CMVideoDimensions;
    fn CMFormatDescriptionGetMediaSubType(desc: *mut c_void) -> u32;
    fn CMSampleBufferGetImageBuffer(sbuf: *mut c_void) -> *mut c_void;
    fn CMSampleBufferGetPresentationTimeStamp(sbuf: *mut c_void) -> CMTime;
    fn CMTimeGetSeconds(time: CMTime) -> f64;
}

// ============================================================================
// FFI 宣言: libdispatch
// ============================================================================

#[link(name = "System", kind = "dylib")]
unsafe extern "C" {
    fn dispatch_queue_create(label: *const i8, attr: *mut c_void) -> *mut c_void;
}

// ============================================================================
// FFI 宣言: AVFoundation (リンクのみ、実際の呼び出しは msg_send 経由)
// ============================================================================

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {}

// ============================================================================
// 定数: CVPixelBuffer
// ============================================================================

/// kCVPixelBufferLock_ReadOnly
const CVPIXELBUFFER_LOCK_READONLY: u64 = 0x00000001;

// ============================================================================
// 定数: ピクセルフォーマット (FourCC)
// ============================================================================

/// NV12 (420v) - Video Range
const PIXEL_FORMAT_420V: u32 = 0x34323076; // '420v'
/// NV12 (420f) - Full Range
const PIXEL_FORMAT_420F: u32 = 0x34323066; // '420f'
/// UYVY (2vuy)
const PIXEL_FORMAT_UYVY: u32 = 0x32767579; // '2vuy'
/// YUY2 (yuvs)
const PIXEL_FORMAT_YUVS: u32 = 0x79757673; // 'yuvs'
/// BGRA
const PIXEL_FORMAT_BGRA: u32 = 0x42475241; // 'BGRA'
/// ARGB (数値)
const PIXEL_FORMAT_ARGB: u32 = 0x00000020;
/// RGB (24bit, 数値)
const PIXEL_FORMAT_RGB24: u32 = 0x00000018;
/// BGR (24BG)
const PIXEL_FORMAT_BGR24: u32 = 0x32344247; // '24BG'

// ============================================================================
// 定数: OBJC_ASSOCIATION_*
// ============================================================================

/// OBJC_ASSOCIATION_RETAIN_NONATOMIC
const OBJC_ASSOCIATION_RETAIN_NONATOMIC: usize = 1;

// ============================================================================
// 型定義: CoreMedia
// ============================================================================

/// CMTime 構造体
#[repr(C)]
#[derive(Copy, Clone)]
struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

impl CMTime {
    /// フレームレートから CMTime を作成
    fn from_fps(fps: u32) -> Self {
        Self {
            value: 1,
            timescale: fps as i32,
            flags: 1, // kCMTimeFlags_Valid
            epoch: 0,
        }
    }
}

/// CMVideoDimensions 構造体
#[repr(C)]
struct CMVideoDimensions {
    width: i32,
    height: i32,
}

// ============================================================================
// 型定義: Objective-C ランタイム
// ============================================================================

/// objc_msgSendSuper 用の構造体
#[repr(C)]
struct ObjcSuper {
    receiver: *mut c_void,
    super_class: *mut c_void,
}

// ============================================================================
// グローバル変数
// ============================================================================

/// objc_setAssociatedObject / objc_getAssociatedObject 用のキー
/// 両方の関数で同じアドレスを使用するためにモジュールレベルで定義
static CONTEXT_KEY: u8 = 0;

/// デリゲートクラスの登録状態
static DELEGATE_CLASS_REGISTERED: std::sync::Once = std::sync::Once::new();

/// 登録されたデリゲートクラス
static DELEGATE_CLASS: Mutex<SendPtr> = Mutex::new(SendPtr(std::ptr::null_mut()));

// ============================================================================
// ヘルパー型
// ============================================================================

/// Send/Sync 可能なポインタラッパー
///
/// Objective-C クラスポインタは一度登録されると不変であり、
/// 複数スレッドから安全に読み取り可能
#[derive(Clone, Copy)]
struct SendPtr(*mut c_void);

unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

/// デリゲートに関連付けるコンテキスト
struct DelegateContext {
    latest_frame: Arc<Mutex<Option<Frame>>>,
}

// ============================================================================
// ヘルパー関数: NSObject 基本操作
// ============================================================================

/// alloc を呼び出す
///
/// # Safety
///
/// class は有効な Objective-C クラスである必要がある
unsafe fn ns_alloc(class: *mut c_void) -> *mut c_void {
    unsafe {
        let sel = sel_registerName(c"alloc".as_ptr());
        msg_send(class, sel)
    }
}

/// init を呼び出す
///
/// # Safety
///
/// obj は alloc で作成された未初期化オブジェクトである必要がある
unsafe fn ns_init(obj: *mut c_void) -> *mut c_void {
    unsafe {
        let sel = sel_registerName(c"init".as_ptr());
        msg_send(obj, sel)
    }
}

/// release を呼び出す
///
/// # Safety
///
/// obj は有効な Objective-C オブジェクトである必要がある
unsafe fn ns_release(obj: *mut c_void) {
    unsafe {
        let sel = sel_registerName(c"release".as_ptr());
        msg_send(obj, sel);
    }
}

/// retain を呼び出す
///
/// # Safety
///
/// obj は有効な Objective-C オブジェクトである必要がある
unsafe fn ns_retain(obj: *mut c_void) -> *mut c_void {
    unsafe {
        let sel = sel_registerName(c"retain".as_ptr());
        msg_send(obj, sel)
    }
}

// ============================================================================
// ヘルパー関数: NSAutoreleasePool
// ============================================================================

/// NSAutoreleasePool を作成
///
/// # Safety
///
/// 返されたプールは drain_autorelease_pool で解放する必要がある
unsafe fn create_autorelease_pool() -> *mut c_void {
    unsafe {
        let class = objc_getClass(c"NSAutoreleasePool".as_ptr());
        let pool = ns_alloc(class);
        ns_init(pool)
    }
}

/// NSAutoreleasePool を drain する
///
/// # Safety
///
/// pool は create_autorelease_pool で作成されたものである必要がある
unsafe fn drain_autorelease_pool(pool: *mut c_void) {
    unsafe {
        let sel = sel_registerName(c"drain".as_ptr());
        msg_send(pool, sel);
    }
}

// ============================================================================
// ヘルパー関数: NSString
// ============================================================================

/// Rust 文字列から NSString を作成
///
/// # Safety
///
/// 返される NSString は autorelease pool で管理される
unsafe fn nsstring_from_str(s: &str) -> *mut c_void {
    unsafe {
        let class = objc_getClass(c"NSString".as_ptr());
        let sel = sel_registerName(c"stringWithUTF8String:".as_ptr());
        let cstring = CString::new(s).unwrap();
        msg_send_cstr(class, sel, cstring.as_ptr())
    }
}

/// NSString から Rust 文字列を取得
///
/// # Safety
///
/// nsstring は有効な NSString オブジェクトまたは null である必要がある
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

// ============================================================================
// ヘルパー関数: NSArray
// ============================================================================

/// NSArray の要素数を取得
///
/// # Safety
///
/// array は有効な NSArray オブジェクトである必要がある
unsafe fn nsarray_count(array: *mut c_void) -> usize {
    unsafe {
        let sel = sel_registerName(c"count".as_ptr());
        msg_send_usize(array, sel)
    }
}

/// NSArray から指定インデックスのオブジェクトを取得
///
/// # Safety
///
/// array は有効な NSArray オブジェクトであり、
/// index は配列の範囲内である必要がある
unsafe fn nsarray_object_at_index(array: *mut c_void, index: usize) -> *mut c_void {
    unsafe {
        let sel = sel_registerName(c"objectAtIndex:".as_ptr());
        msg_send_usize_arg(array, sel, index)
    }
}

// ============================================================================
// ヘルパー関数: AVFrameRateRange
// ============================================================================

/// 最大フレームレートを取得
///
/// # Safety
///
/// range は有効な AVFrameRateRange オブジェクトである必要がある
unsafe fn get_frame_rate_range_max(range: *mut c_void) -> f64 {
    unsafe {
        let sel = sel_registerName(c"maxFrameRate".as_ptr());

        #[cfg(target_arch = "x86_64")]
        {
            objc_msgSend_fpret(range, sel)
        }

        #[cfg(target_arch = "aarch64")]
        {
            msg_send_f64(range, sel)
        }
    }
}

/// 最小フレームレートを取得
///
/// # Safety
///
/// range は有効な AVFrameRateRange オブジェクトである必要がある
unsafe fn get_frame_rate_range_min(range: *mut c_void) -> f64 {
    unsafe {
        let sel = sel_registerName(c"minFrameRate".as_ptr());

        #[cfg(target_arch = "x86_64")]
        {
            objc_msgSend_fpret(range, sel)
        }

        #[cfg(target_arch = "aarch64")]
        {
            msg_send_f64(range, sel)
        }
    }
}

// ============================================================================
// ヘルパー関数: CMFormatDescription
// ============================================================================

/// 解像度を取得
///
/// # Safety
///
/// format_desc は有効な CMFormatDescription である必要がある
unsafe fn get_format_dimensions(format_desc: *mut c_void) -> (u32, u32) {
    unsafe {
        let dims = CMVideoFormatDescriptionGetDimensions(format_desc);
        (dims.width as u32, dims.height as u32)
    }
}

/// メディアサブタイプ (ピクセルフォーマット) を取得
///
/// # Safety
///
/// format_desc は有効な CMFormatDescription である必要がある
unsafe fn get_format_media_subtype(format_desc: *mut c_void) -> u32 {
    unsafe { CMFormatDescriptionGetMediaSubType(format_desc) }
}

// ============================================================================
// ヘルパー関数: AVCaptureDevice
// ============================================================================

/// uniqueID から AVCaptureDevice を取得
///
/// # Safety
///
/// この関数内で Objective-C オブジェクトが作成される
unsafe fn get_av_capture_device(device_id: &str) -> Result<*mut c_void, UvcError> {
    unsafe {
        let device_class = objc_getClass(c"AVCaptureDevice".as_ptr());
        let sel = sel_registerName(c"deviceWithUniqueID:".as_ptr());
        let device_id_nsstring = nsstring_from_str(device_id);
        let device = msg_send_ptr(device_class, sel, device_id_nsstring);

        if device.is_null() {
            return Err(UvcError::PlatformError(format!(
                "デバイスが見つかりません: {}",
                device_id
            )));
        }

        Ok(device)
    }
}

// ============================================================================
// ヘルパー関数: dispatch_queue
// ============================================================================

/// キャプチャ用の dispatch_queue を作成
///
/// # Safety
///
/// 返されたキューは適切に解放する必要がある
unsafe fn create_dispatch_queue() -> *mut c_void {
    unsafe { dispatch_queue_create(c"uvc.capture".as_ptr(), std::ptr::null_mut()) }
}

// ============================================================================
// ヘルパー関数: ピクセルフォーマット変換
// ============================================================================

/// CoreVideo ピクセルフォーマットから Format へ変換
fn pixel_format_to_format(pixel_format: u32) -> Option<Format> {
    match pixel_format {
        PIXEL_FORMAT_420V | PIXEL_FORMAT_420F => Some(Format::NV12),
        PIXEL_FORMAT_UYVY | PIXEL_FORMAT_YUVS => Some(Format::YUY2),
        PIXEL_FORMAT_BGRA | PIXEL_FORMAT_ARGB => Some(Format::RGBA),
        PIXEL_FORMAT_RGB24 | PIXEL_FORMAT_BGR24 => Some(Format::RGB),
        _ => None,
    }
}

/// Format から CoreVideo ピクセルフォーマットへ変換 (キャプチャ出力用)
fn format_to_pixel_format(format: Format) -> u32 {
    match format {
        Format::NV12 => PIXEL_FORMAT_420F,
        Format::YUY2 => PIXEL_FORMAT_YUVS,
        Format::RGBA | Format::RGB | Format::MJPEG => PIXEL_FORMAT_BGRA,
    }
}

/// フォーマットがマッチするか判定
fn does_format_match(pixel_format: u32, capture_format: Format) -> bool {
    match capture_format {
        Format::NV12 => pixel_format == PIXEL_FORMAT_420V || pixel_format == PIXEL_FORMAT_420F,
        Format::YUY2 => pixel_format == PIXEL_FORMAT_UYVY || pixel_format == PIXEL_FORMAT_YUVS,
        Format::RGBA => pixel_format == PIXEL_FORMAT_BGRA || pixel_format == PIXEL_FORMAT_ARGB,
        Format::RGB => pixel_format == PIXEL_FORMAT_RGB24 || pixel_format == PIXEL_FORMAT_BGR24,
        Format::MJPEG => false,
    }
}

// ============================================================================
// 公開 API: CVPixelBuffer 操作 (frame.rs から使用)
// ============================================================================

/// CVPixelBuffer を retain する
///
/// # Safety
///
/// buffer は有効な CVPixelBufferRef である必要がある
pub unsafe fn cvpixelbuffer_retain(buffer: *mut c_void) {
    unsafe { CVPixelBufferRetain(buffer) };
}

/// CVPixelBuffer を release する
///
/// # Safety
///
/// buffer は有効な CVPixelBufferRef である必要がある
pub unsafe fn cvpixelbuffer_release(buffer: *mut c_void) {
    unsafe { CVPixelBufferRelease(buffer) };
}

// ============================================================================
// 公開 API: デバイス列挙
// ============================================================================

/// 利用可能なカメラデバイスを列挙
pub fn list_devices() -> Result<Vec<DeviceInfo>, UvcError> {
    unsafe {
        let pool = create_autorelease_pool();

        // AVCaptureDeviceDiscoverySession クラスを取得
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

        let sel = sel_registerName(c"arrayWithObjects:count:".as_ptr());
        let device_types: [*mut c_void; 2] = [built_in_type, external_type];
        let device_types_array =
            msg_send_arr_count(nsarray_class, sel, device_types.as_ptr(), 2usize);

        // メディアタイプ: video
        let video_media_type = nsstring_from_str("vide");

        // ディスカバリセッションを作成
        let sel = sel_registerName(c"discoverySessionWithDeviceTypes:mediaType:position:".as_ptr());
        // AVCaptureDevicePositionUnspecified = 0
        let discovery_session = msg_send_obj_obj_i64(
            discovery_class,
            sel,
            device_types_array,
            video_media_type,
            0i64,
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

            // localizedName
            let localized_name_sel = sel_registerName(c"localizedName".as_ptr());
            let name_nsstring = msg_send(device, localized_name_sel);
            let name = nsstring_to_string(name_nsstring);

            // uniqueID
            let unique_id_sel = sel_registerName(c"uniqueID".as_ptr());
            let unique_id_nsstring = msg_send(device, unique_id_sel);
            let unique_id = nsstring_to_string(unique_id_nsstring);

            result.push(DeviceInfo::new(name, unique_id, i as u32));
        }

        drain_autorelease_pool(pool);
        Ok(result)
    }
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
    let device_id = info.unique_id.clone();

    Ok(Box::new(DeviceMacOS::new(info, device_id)))
}

// ============================================================================
// DeviceMacOS 実装
// ============================================================================

/// macOS デバイス実装
pub struct DeviceMacOS {
    info: DeviceInfo,
    device_id: String,
    running: Arc<RwLock<bool>>,
    latest_frame: Arc<Mutex<Option<Frame>>>,
    on_connected: Arc<Mutex<Option<DeviceCallback>>>,
    on_disconnected: Arc<Mutex<Option<DeviceCallback>>>,
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

        start_capture_session(
            &self.device_id,
            width,
            height,
            fps,
            capture_format,
            self.latest_frame.clone(),
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

// ============================================================================
// 内部関数: フォーマット取得
// ============================================================================

/// デバイスのサポートフォーマットを取得
fn get_device_formats(device_id: &str) -> Result<Vec<FormatInfo>, UvcError> {
    unsafe {
        let device = get_av_capture_device(device_id)?;

        let formats_sel = sel_registerName(c"formats".as_ptr());
        let formats = msg_send(device, formats_sel);

        let count = nsarray_count(formats);
        let mut result = Vec::new();

        for i in 0..count {
            let format = nsarray_object_at_index(formats, i);

            let format_desc_sel = sel_registerName(c"formatDescription".as_ptr());
            let format_desc = msg_send(format, format_desc_sel);

            let (width, height) = get_format_dimensions(format_desc);
            let pixel_format = get_format_media_subtype(format_desc);

            if let Some(fmt) = pixel_format_to_format(pixel_format) {
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

// ============================================================================
// 内部関数: キャプチャセッション
// ============================================================================

/// キャプチャセッションを開始
#[allow(clippy::too_many_arguments)]
fn start_capture_session(
    device_id: &str,
    width: u32,
    height: u32,
    fps: u32,
    capture_format: Format,
    latest_frame: Arc<Mutex<Option<Frame>>>,
    session_holder: Arc<Mutex<Option<*mut c_void>>>,
    delegate_holder: Arc<Mutex<Option<*mut c_void>>>,
) -> Result<(), UvcError> {
    unsafe {
        let device = get_av_capture_device(device_id)?;

        // AVCaptureSession を作成
        let session_class = objc_getClass(c"AVCaptureSession".as_ptr());
        let session = ns_alloc(session_class);
        let session = ns_init(session);

        // 設定開始
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

        // 遅延フレームを破棄
        let set_discards_sel = sel_registerName(c"setAlwaysDiscardsLateVideoFrames:".as_ptr());
        msg_send_i32_arg(output, set_discards_sel, 1i32);

        // ビデオ設定
        let pixel_format = format_to_pixel_format(capture_format);
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

        // 設定完了
        let commit_sel = sel_registerName(c"commitConfiguration".as_ptr());
        msg_send(session, commit_sel);

        // キャプチャ開始
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

/// キャプチャセッションを停止
fn stop_capture_session(
    session_holder: Arc<Mutex<Option<*mut c_void>>>,
    delegate_holder: Arc<Mutex<Option<*mut c_void>>>,
) -> Result<(), UvcError> {
    unsafe {
        if let Some(session) = session_holder.lock().unwrap().take() {
            let stop_running_sel = sel_registerName(c"stopRunning".as_ptr());
            msg_send(session, stop_running_sel);
            ns_release(session);
        }

        if let Some(delegate) = delegate_holder.lock().unwrap().take() {
            ns_release(delegate);
        }

        Ok(())
    }
}

// ============================================================================
// 内部関数: フォーマット選択
// ============================================================================

/// 指定された条件に最も適合するフォーマットを探す
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
            if !does_format_match(pixel_format, capture_format) {
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

                // 許容誤差 0.5fps
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

/// フレーム期間を設定
unsafe fn set_frame_duration(device: *mut c_void, fps: u32) {
    let frame_duration = CMTime::from_fps(fps);

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
        let number_class = objc_getClass(c"NSNumber".as_ptr());
        let number_with_uint_sel = sel_registerName(c"numberWithUnsignedInt:".as_ptr());

        // PixelFormatType
        let pixel_format_key = nsstring_from_str("PixelFormatType");
        let pixel_format_value = msg_send_u32_arg(number_class, number_with_uint_sel, pixel_format);
        msg_send_obj_obj(dict, set_object_sel, pixel_format_value, pixel_format_key);

        // Width
        let width_key = nsstring_from_str("Width");
        let width_value = msg_send_u32_arg(number_class, number_with_uint_sel, width);
        msg_send_obj_obj(dict, set_object_sel, width_value, width_key);

        // Height
        let height_key = nsstring_from_str("Height");
        let height_value = msg_send_u32_arg(number_class, number_with_uint_sel, height);
        msg_send_obj_obj(dict, set_object_sel, height_value, height_key);

        dict
    }
}

// ============================================================================
// 内部関数: デリゲート
// ============================================================================

/// サンプルバッファデリゲートを作成
unsafe fn create_sample_buffer_delegate(latest_frame: Arc<Mutex<Option<Frame>>>) -> *mut c_void {
    // デリゲートクラスを一度だけ登録
    DELEGATE_CLASS_REGISTERED.call_once(|| {
        unsafe { register_delegate_class() };
    });

    unsafe {
        let delegate_class = DELEGATE_CLASS.lock().unwrap().0;
        let delegate = ns_alloc(delegate_class);
        let delegate = ns_init(delegate);

        // コンテキストを作成して関連付け
        let context = Box::new(DelegateContext { latest_frame });
        let context_ptr = Box::into_raw(context);

        // NSNumber でポインタをラップ
        let number_class = objc_getClass(c"NSNumber".as_ptr());
        let number_with_ptr_sel = sel_registerName(c"numberWithUnsignedLongLong:".as_ptr());
        let context_number =
            msg_send_u64_arg(number_class, number_with_ptr_sel, context_ptr as u64);

        objc_setAssociatedObject(
            delegate,
            &CONTEXT_KEY as *const _ as *const c_void,
            context_number,
            OBJC_ASSOCIATION_RETAIN_NONATOMIC,
        );

        delegate
    }
}

/// デリゲートクラスを登録
unsafe fn register_delegate_class() {
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
            capture_output_callback as *mut c_void,
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

// ============================================================================
// コールバック関数
// ============================================================================

/// キャプチャ出力コールバック
///
/// AVCaptureVideoDataOutputSampleBufferDelegate の
/// captureOutput:didOutputSampleBuffer:fromConnection: 実装
extern "C" fn capture_output_callback(
    this: *mut c_void,
    _sel: *mut c_void,
    _output: *mut c_void,
    sample_buffer: *mut c_void,
    _connection: *mut c_void,
) {
    unsafe {
        // コンテキストを取得
        let context_number =
            objc_getAssociatedObject(this, &CONTEXT_KEY as *const _ as *const c_void);
        if context_number.is_null() {
            return;
        }

        let sel = sel_registerName(c"unsignedLongLongValue".as_ptr());
        let context_ptr = msg_send(context_number, sel) as *mut DelegateContext;
        if context_ptr.is_null() {
            return;
        }

        let context = &*context_ptr;

        // CMSampleBuffer から CVImageBuffer を取得
        let image_buffer = CMSampleBufferGetImageBuffer(sample_buffer);
        if image_buffer.is_null() {
            return;
        }

        // ピクセルバッファをロック
        let lock_result = CVPixelBufferLockBaseAddress(image_buffer, CVPIXELBUFFER_LOCK_READONLY);
        if lock_result != 0 {
            return;
        }

        let width = CVPixelBufferGetWidth(image_buffer) as u32;
        let height = CVPixelBufferGetHeight(image_buffer) as u32;
        let pixel_format = CVPixelBufferGetPixelFormatType(image_buffer);

        // フォーマットを判定
        let format = match pixel_format_to_format(pixel_format) {
            Some(fmt) => fmt,
            None => {
                CVPixelBufferUnlockBaseAddress(image_buffer, CVPIXELBUFFER_LOCK_READONLY);
                return;
            }
        };

        // CVPixelBuffer を retain
        CVPixelBufferRetain(image_buffer);

        // リリース関数を作成
        let buffer_addr = image_buffer as usize;
        let release_fn: NativeBufferReleaseFn = Box::new(move || {
            let buffer = buffer_addr as *mut c_void;
            CVPixelBufferUnlockBaseAddress(buffer, CVPIXELBUFFER_LOCK_READONLY);
            CVPixelBufferRelease(buffer);
        });

        // タイムスタンプを取得 (マイクロ秒)
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
        let context_number =
            objc_getAssociatedObject(this, &CONTEXT_KEY as *const _ as *const c_void);
        if !context_number.is_null() {
            let sel = sel_registerName(c"unsignedLongLongValue".as_ptr());
            let context_ptr = msg_send(context_number, sel) as *mut DelegateContext;
            if !context_ptr.is_null() {
                drop(Box::from_raw(context_ptr));
            }
        }

        // スーパークラスの dealloc を呼ぶ
        let super_class = objc_getClass(c"NSObject".as_ptr());
        let dealloc_sel = sel_registerName(c"dealloc".as_ptr());

        let mut super_struct = ObjcSuper {
            receiver: this,
            super_class,
        };
        objc_msgSendSuper(&mut super_struct as *mut _, dealloc_sel);
    }
}
