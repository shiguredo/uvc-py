#include "uvc.h"

#import <AVFoundation/AVFoundation.h>
#import <CoreMedia/CoreMedia.h>
#import <CoreVideo/CoreVideo.h>
#import <Foundation/Foundation.h>

#include <mutex>

// AVFoundation デリゲート（グローバルスコープで定義）
@interface UVCCaptureDelegate : NSObject <AVCaptureVideoDataOutputSampleBufferDelegate>
@property(nonatomic) std::shared_ptr<uvc::Frame>* latest_frame;
@property(nonatomic) std::mutex* frame_mutex;
@property(nonatomic, copy) void (^on_connected_block)(void);
@property(nonatomic, copy) void (^on_disconnected_block)(void);

- (void)handleDeviceConnected:(NSNotification*)notification;
- (void)handleDeviceDisconnected:(NSNotification*)notification;
@end

@implementation UVCCaptureDelegate

- (void)captureOutput:(AVCaptureOutput*)output
    didOutputSampleBuffer:(CMSampleBufferRef)sampleBuffer
           fromConnection:(AVCaptureConnection*)connection {
  CVImageBufferRef imageBuffer = CMSampleBufferGetImageBuffer(sampleBuffer);
  if (!imageBuffer) {
    return;
  }
  CVPixelBufferLockBaseAddress(imageBuffer, kCVPixelBufferLock_ReadOnly);

  size_t width = CVPixelBufferGetWidth(imageBuffer);
  size_t height = CVPixelBufferGetHeight(imageBuffer);
  OSType pixelFormat = CVPixelBufferGetPixelFormatType(imageBuffer);

  std::shared_ptr<uvc::Frame> frame;

  // CVPixelBuffer を retain してゼロコピーで参照
  CVPixelBufferRetain(imageBuffer);

  // デストラクタで unlock + release する関数
  auto release_func = [](void* buffer) {
    CVPixelBufferRef pb = static_cast<CVPixelBufferRef>(buffer);
    CVPixelBufferUnlockBaseAddress(pb, kCVPixelBufferLock_ReadOnly);
    CVPixelBufferRelease(pb);
  };

  // サポートするフォーマット: NV12, YUY2, UYVY
  if (pixelFormat == kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange ||
      pixelFormat == kCVPixelFormatType_420YpCbCr8BiPlanarFullRange) {
    // NV12 フォーマット
    frame = std::make_shared<uvc::Frame>(static_cast<uint32_t>(width),
                                         static_cast<uint32_t>(height),
                                         uvc::Format::NV12);
    frame->set_native_buffer(imageBuffer, release_func);

    uint8_t* y_src = static_cast<uint8_t*>(CVPixelBufferGetBaseAddressOfPlane(imageBuffer, 0));
    size_t y_stride = CVPixelBufferGetBytesPerRowOfPlane(imageBuffer, 0);
    uint8_t* uv_src = static_cast<uint8_t*>(CVPixelBufferGetBaseAddressOfPlane(imageBuffer, 1));
    size_t uv_stride = CVPixelBufferGetBytesPerRowOfPlane(imageBuffer, 1);
    frame->set_nv12_planes(y_src, y_stride, uv_src, uv_stride);

  } else if (pixelFormat == kCVPixelFormatType_422YpCbCr8_yuvs) {
    // YUY2 フォーマット (Y0 U0 Y1 V0)
    frame = std::make_shared<uvc::Frame>(static_cast<uint32_t>(width),
                                         static_cast<uint32_t>(height),
                                         uvc::Format::YUY2);
    frame->set_native_buffer(imageBuffer, release_func);

    uint8_t* src = static_cast<uint8_t*>(CVPixelBufferGetBaseAddress(imageBuffer));
    size_t bytesPerRow = CVPixelBufferGetBytesPerRow(imageBuffer);
    frame->set_packed_plane(src, bytesPerRow);

  } else if (pixelFormat == kCVPixelFormatType_422YpCbCr8) {
    // UYVY フォーマット (U0 Y0 V0 Y1)
    frame = std::make_shared<uvc::Frame>(static_cast<uint32_t>(width),
                                         static_cast<uint32_t>(height),
                                         uvc::Format::UYVY);
    frame->set_native_buffer(imageBuffer, release_func);

    uint8_t* src = static_cast<uint8_t*>(CVPixelBufferGetBaseAddress(imageBuffer));
    size_t bytesPerRow = CVPixelBufferGetBytesPerRow(imageBuffer);
    frame->set_packed_plane(src, bytesPerRow);

  } else {
    // 未対応フォーマット
    CVPixelBufferUnlockBaseAddress(imageBuffer, kCVPixelBufferLock_ReadOnly);
    CVPixelBufferRelease(imageBuffer);
    return;
  }

  CMTime pts = CMSampleBufferGetPresentationTimeStamp(sampleBuffer);
  frame->set_timestamp(
      static_cast<uint64_t>(CMTimeGetSeconds(pts) * 1000000));

  // 最新フレームのみを保持（レイテンシ削減）
  std::lock_guard<std::mutex> lock(*self.frame_mutex);
  *self.latest_frame = frame;
}

- (void)handleDeviceConnected:(NSNotification*)notification {
  if (self.on_connected_block) {
    self.on_connected_block();
  }
}

- (void)handleDeviceDisconnected:(NSNotification*)notification {
  if (self.on_disconnected_block) {
    self.on_disconnected_block();
  }
}

@end

namespace uvc {

// macOS デバイス実装
class DeviceMacOS : public Device {
 public:
  DeviceMacOS(const DeviceInfo& info, AVCaptureDevice* device)
      : info_(info), device_(device) {
    delegate_ = [[UVCCaptureDelegate alloc] init];
    delegate_.latest_frame = &latest_frame_;
    delegate_.frame_mutex = &frame_mutex_;
  }

  ~DeviceMacOS() override { stop(); }

  void start(uint32_t width, uint32_t height, uint32_t fps,
             Format capture_format, std::optional<Format> output_format_opt) override {
    // output_format_opt は現在未使用（将来の拡張用）
    (void)output_format_opt;

    if (running_) return;

    @autoreleasepool {
      @try {
        NSError* error = nil;

        // セッション作成
        session_ = [[AVCaptureSession alloc] init];

      // セッション設定開始
      [session_ beginConfiguration];

      // まずデバイスのフォーマットを設定
      [device_ lockForConfiguration:&error];
      if (error) {
        [session_ commitConfiguration];
        throw std::runtime_error("Failed to lock device: " +
                                 std::string([error.localizedDescription UTF8String]));
      }

      // 最適なフォーマットを探す
      AVCaptureDeviceFormat* bestFormat = nil;
      AVFrameRateRange* bestFrameRateRange = nil;
      double bestRangeWidth = DBL_MAX;

      for (AVCaptureDeviceFormat* f in device_.formats) {
        CMVideoDimensions dims =
            CMVideoFormatDescriptionGetDimensions(f.formatDescription);
        if (dims.width == (int32_t)width && dims.height == (int32_t)height) {
          // ピクセルフォーマットをチェック
          CMFormatDescriptionRef desc = f.formatDescription;
          FourCharCode mediaSubType = CMFormatDescriptionGetMediaSubType(desc);

          // フォーマットが一致するか確認
          bool formatMatch = false;
          if (capture_format == Format::NV12) {
            formatMatch = (mediaSubType == kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange ||
                          mediaSubType == kCVPixelFormatType_420YpCbCr8BiPlanarFullRange);
          } else if (capture_format == Format::YUY2) {
            formatMatch = (mediaSubType == kCVPixelFormatType_422YpCbCr8_yuvs);
          } else if (capture_format == Format::UYVY) {
            formatMatch = (mediaSubType == kCVPixelFormatType_422YpCbCr8);
          } else if (capture_format == Format::RGBA) {
            formatMatch = (mediaSubType == kCVPixelFormatType_32BGRA ||
                          mediaSubType == kCVPixelFormatType_32ARGB);
          } else if (capture_format == Format::RGB) {
            formatMatch = (mediaSubType == kCVPixelFormatType_24RGB ||
                          mediaSubType == kCVPixelFormatType_24BGR);
          }

          if (!formatMatch) continue;

          // フレームレートもチェック（指定 fps が範囲内にあるか）
          for (AVFrameRateRange* range in f.videoSupportedFrameRateRanges) {
            double minFps = range.minFrameRate;
            double maxFps = range.maxFrameRate;
            // 指定 fps が範囲内にあるか（0.5 の許容誤差）
            if ((double)fps >= minFps - 0.5 && (double)fps <= maxFps + 0.5) {
              // より狭い範囲を優先（正確に一致する範囲が最優先）
              double rangeWidth = maxFps - minFps;
              if (rangeWidth < bestRangeWidth) {
                bestFormat = f;
                bestFrameRateRange = range;
                bestRangeWidth = rangeWidth;
              }
            }
          }
        }
      }

      if (!bestFormat) {
        [device_ unlockForConfiguration];
        [session_ commitConfiguration];
        std::string fmt_str;
        switch (capture_format) {
          case Format::MJPEG: fmt_str = "MJPEG (not supported)"; break;
          case Format::YUY2: fmt_str = "YUY2"; break;
          case Format::UYVY: fmt_str = "UYVY"; break;
          case Format::NV12: fmt_str = "NV12"; break;
          case Format::RGB: fmt_str = "RGB"; break;
          case Format::RGBA: fmt_str = "RGBA"; break;
          case Format::BGRA: fmt_str = "BGRA"; break;
        }
        throw std::runtime_error("Unsupported format: " +
                                 std::to_string(width) + "x" +
                                 std::to_string(height) + "@" +
                                 std::to_string(fps) + " " + fmt_str);
      }

      device_.activeFormat = bestFormat;
      // bestFrameRateRange から正確なフレームレートを設定
      CMTime frameDuration = bestFrameRateRange.minFrameDuration;
      device_.activeVideoMinFrameDuration = frameDuration;
      device_.activeVideoMaxFrameDuration = frameDuration;

      [device_ unlockForConfiguration];

      // 入力設定
      AVCaptureDeviceInput* input =
          [AVCaptureDeviceInput deviceInputWithDevice:device_ error:&error];
      if (error) {
        [session_ commitConfiguration];
        throw std::runtime_error("Failed to create device input: " +
                                 std::string([error.localizedDescription UTF8String]));
      }

      if ([session_ canAddInput:input]) {
        [session_ addInput:input];
      }

      // 出力設定
      AVCaptureVideoDataOutput* output = [[AVCaptureVideoDataOutput alloc] init];
      output.alwaysDiscardsLateVideoFrames = YES;

      // 出力フォーマット設定
      // キャプチャフォーマットに応じて適切なピクセルフォーマットを設定
      OSType outputPixelFormat;
      if (capture_format == Format::NV12) {
        outputPixelFormat = kCVPixelFormatType_420YpCbCr8BiPlanarFullRange;
      } else if (capture_format == Format::YUY2) {
        outputPixelFormat = kCVPixelFormatType_422YpCbCr8_yuvs;
      } else if (capture_format == Format::UYVY) {
        outputPixelFormat = kCVPixelFormatType_422YpCbCr8;
      } else {
        outputPixelFormat = kCVPixelFormatType_32BGRA;
      }
      output.videoSettings = @{
        (NSString*)kCVPixelBufferPixelFormatTypeKey : @(outputPixelFormat),
        (NSString*)kCVPixelBufferWidthKey : @(width),
        (NSString*)kCVPixelBufferHeightKey : @(height),
      };

      dispatch_queue_t queue =
          dispatch_queue_create("uvc.capture", DISPATCH_QUEUE_SERIAL);
      [output setSampleBufferDelegate:delegate_ queue:queue];

      if ([session_ canAddOutput:output]) {
        [session_ addOutput:output];
      }

      // セッション設定完了
      [session_ commitConfiguration];

      // デバイス接続/切断通知を購読
      [[NSNotificationCenter defaultCenter]
          addObserver:delegate_
             selector:@selector(handleDeviceConnected:)
                 name:AVCaptureDeviceWasConnectedNotification
               object:device_];
      [[NSNotificationCenter defaultCenter]
          addObserver:delegate_
             selector:@selector(handleDeviceDisconnected:)
                 name:AVCaptureDeviceWasDisconnectedNotification
               object:device_];

      [session_ startRunning];

      // セッション開始後にフレームレートを再設定
      // （セッション開始時にリセットされる場合があるため）
      if ([device_ lockForConfiguration:&error]) {
        device_.activeVideoMinFrameDuration = frameDuration;
        device_.activeVideoMaxFrameDuration = frameDuration;
        [device_ unlockForConfiguration];
      }

      running_ = true;
      }
      @catch (NSException* exception) {
        throw std::runtime_error("AVFoundation exception: " +
                                 std::string([exception.name UTF8String]) + " - " +
                                 std::string([exception.reason UTF8String]));
      }
    }
  }

  void stop() override {
    if (!running_) return;

    @autoreleasepool {
      // デバイス通知の購読解除
      [[NSNotificationCenter defaultCenter]
          removeObserver:delegate_
                    name:AVCaptureDeviceWasConnectedNotification
                  object:device_];
      [[NSNotificationCenter defaultCenter]
          removeObserver:delegate_
                    name:AVCaptureDeviceWasDisconnectedNotification
                  object:device_];

      [session_ stopRunning];
      session_ = nil;
      running_ = false;

      std::lock_guard<std::mutex> lock(frame_mutex_);
      latest_frame_ = nullptr;
    }
  }

  std::shared_ptr<Frame> get_frame() override {
    std::lock_guard<std::mutex> lock(frame_mutex_);
    auto frame = latest_frame_;
    latest_frame_ = nullptr;
    return frame;
  }

  bool is_running() const override { return running_; }

  const DeviceInfo& info() const override { return info_; }

  std::vector<FormatInfo> get_supported_formats() const override {
    std::vector<FormatInfo> formats;

    @autoreleasepool {
      for (AVCaptureDeviceFormat* format in device_.formats) {
        CMVideoDimensions dims =
            CMVideoFormatDescriptionGetDimensions(format.formatDescription);

        // ピクセルフォーマットを取得
        CMFormatDescriptionRef desc = format.formatDescription;
        FourCharCode mediaSubType = CMFormatDescriptionGetMediaSubType(desc);

        // サポートするフォーマット: NV12, YUY2, UYVY
        Format fmt;
        switch (mediaSubType) {
          case kCVPixelFormatType_422YpCbCr8_yuvs:  // 'yuvs' / YUY2
            fmt = Format::YUY2;
            break;
          case kCVPixelFormatType_422YpCbCr8:  // '2vuy' / UYVY
            fmt = Format::UYVY;
            break;
          case kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange:  // '420v' / NV12
          case kCVPixelFormatType_420YpCbCr8BiPlanarFullRange:   // '420f'
            fmt = Format::NV12;
            break;
          default:
            // MJPEG, BGRA, RGB などの未対応フォーマットはスキップ
            continue;
        }

        for (AVFrameRateRange* range in format.videoSupportedFrameRateRanges) {
          FormatInfo info;
          info.width = static_cast<uint32_t>(dims.width);
          info.height = static_cast<uint32_t>(dims.height);
          info.fps = static_cast<uint32_t>(range.maxFrameRate);
          info.format = fmt;
          formats.push_back(info);
        }
      }
    }

    return formats;
  }

  void set_on_connected(DeviceCallback callback) override {
    on_connected_ = std::move(callback);
    if (on_connected_) {
      delegate_.on_connected_block = ^{
        on_connected_();
      };
    } else {
      delegate_.on_connected_block = nil;
    }
  }

  void set_on_disconnected(DeviceCallback callback) override {
    on_disconnected_ = std::move(callback);
    if (on_disconnected_) {
      delegate_.on_disconnected_block = ^{
        on_disconnected_();
      };
    } else {
      delegate_.on_disconnected_block = nil;
    }
  }

 private:
  DeviceInfo info_;
  AVCaptureDevice* device_;
  AVCaptureSession* session_ = nil;
  UVCCaptureDelegate* delegate_ = nil;
  std::shared_ptr<Frame> latest_frame_;
  std::mutex frame_mutex_;
  bool running_ = false;
  DeviceCallback on_connected_;
  DeviceCallback on_disconnected_;
};

// デバイス列挙
std::vector<DeviceInfo> list_devices_impl() {
  std::vector<DeviceInfo> devices;

  @autoreleasepool {
    AVCaptureDeviceDiscoverySession* discoverySession =
        [AVCaptureDeviceDiscoverySession
            discoverySessionWithDeviceTypes:@[ AVCaptureDeviceTypeBuiltInWideAngleCamera,
                                               AVCaptureDeviceTypeExternal ]
                                  mediaType:AVMediaTypeVideo
                                   position:AVCaptureDevicePositionUnspecified];

    uint32_t index = 0;
    for (AVCaptureDevice* device in discoverySession.devices) {
      DeviceInfo info;
      info.name = std::string([device.localizedName UTF8String]);
      info.unique_id = std::string([device.uniqueID UTF8String]);
      info.index = index++;
      devices.push_back(info);
    }
  }

  return devices;
}

// デバイスオープン
std::shared_ptr<Device> open_device_impl(uint32_t index) {
  @autoreleasepool {
    AVCaptureDeviceDiscoverySession* discoverySession =
        [AVCaptureDeviceDiscoverySession
            discoverySessionWithDeviceTypes:@[ AVCaptureDeviceTypeBuiltInWideAngleCamera,
                                               AVCaptureDeviceTypeExternal ]
                                  mediaType:AVMediaTypeVideo
                                   position:AVCaptureDevicePositionUnspecified];

    if (index >= discoverySession.devices.count) {
      throw std::runtime_error("Device index out of range");
    }

    AVCaptureDevice* device = discoverySession.devices[index];
    DeviceInfo info;
    info.name = std::string([device.localizedName UTF8String]);
    info.unique_id = std::string([device.uniqueID UTF8String]);
    info.index = index;

    return std::make_shared<DeviceMacOS>(info, device);
  }
}

std::shared_ptr<Device> open_device_impl(const DeviceInfo& info) {
  return open_device_impl(info.index);
}

}  // namespace uvc
