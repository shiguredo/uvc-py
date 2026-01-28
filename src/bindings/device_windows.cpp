#include "uvc.h"

#include <mfapi.h>
#include <mferror.h>
#include <mfidl.h>
#include <mfreadwrite.h>

#include <atomic>
#include <mutex>
#include <thread>
#include <vector>

#pragma comment(lib, "mf.lib")
#pragma comment(lib, "mfplat.lib")
#pragma comment(lib, "mfuuid.lib")
#pragma comment(lib, "mfreadwrite.lib")

namespace uvc {

// COM スマートポインタヘルパー
template <class T>
void SafeRelease(T** ppT) {
  if (*ppT) {
    (*ppT)->Release();
    *ppT = nullptr;
  }
}

// Media Foundation 初期化管理
class MFInitializer {
 public:
  static MFInitializer& instance() {
    static MFInitializer inst;
    return inst;
  }

  bool is_initialized() const { return initialized_; }

 private:
  MFInitializer() {
    HRESULT hr = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    // RPC_E_CHANGED_MODE の場合は他のスレッドが COM を初期化済み
    // この場合は CoUninitialize() を呼ばない
    com_initialized_ = SUCCEEDED(hr);
    if (SUCCEEDED(hr) || hr == RPC_E_CHANGED_MODE) {
      hr = MFStartup(MF_VERSION);
      if (SUCCEEDED(hr)) {
        initialized_ = true;
      }
    }
  }

  ~MFInitializer() {
    if (initialized_) {
      MFShutdown();
    }
    if (com_initialized_) {
      CoUninitialize();
    }
  }

  bool initialized_ = false;
  bool com_initialized_ = false;
};

// Windows デバイス実装
class DeviceWindows : public Device {
 public:
  DeviceWindows(const DeviceInfo& info, IMFActivate* activate)
      : info_(info), activate_(activate) {
    if (activate_) {
      activate_->AddRef();
    }
  }

  ~DeviceWindows() override {
    stop();
    SafeRelease(&activate_);
  }

  void start(uint32_t width,
             uint32_t height,
             uint32_t fps,
             Format capture_format,
             std::optional<Format> output_format_opt) override {
    if (running_)
      return;

    // Windows では MJPEG 非対応
    if (capture_format == Format::MJPEG) {
      throw std::runtime_error(
          "MJPEG is not supported on Windows. Use NV12 or YUY2.");
    }

    // output_format は Windows では無視 (capture_format と同じになる)
    (void)output_format_opt;

    HRESULT hr;

    // MediaSource を取得
    hr = activate_->ActivateObject(IID_PPV_ARGS(&media_source_));
    if (FAILED(hr)) {
      throw std::runtime_error("Failed to activate media source");
    }

    // SourceReader を作成
    IMFAttributes* attributes = nullptr;
    hr = MFCreateAttributes(&attributes, 1);
    if (FAILED(hr)) {
      cleanup();
      throw std::runtime_error("Failed to create attributes");
    }

    hr = MFCreateSourceReaderFromMediaSource(media_source_, attributes,
                                             &source_reader_);
    SafeRelease(&attributes);
    if (FAILED(hr)) {
      cleanup();
      throw std::runtime_error("Failed to create source reader");
    }

    // メディアタイプを設定
    IMFMediaType* media_type = nullptr;
    hr = MFCreateMediaType(&media_type);
    if (FAILED(hr)) {
      cleanup();
      throw std::runtime_error("Failed to create media type");
    }

    hr = media_type->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video);
    if (FAILED(hr)) {
      SafeRelease(&media_type);
      cleanup();
      throw std::runtime_error("Failed to set major type");
    }

    // フォーマットを設定
    GUID subtype;
    if (capture_format == Format::NV12) {
      subtype = MFVideoFormat_NV12;
    } else if (capture_format == Format::YUY2) {
      subtype = MFVideoFormat_YUY2;
    } else if (capture_format == Format::UYVY) {
      subtype = MFVideoFormat_UYVY;
    } else {
      SafeRelease(&media_type);
      cleanup();
      throw std::runtime_error("Unsupported format. Use NV12, YUY2, or UYVY.");
    }

    hr = media_type->SetGUID(MF_MT_SUBTYPE, subtype);
    if (FAILED(hr)) {
      SafeRelease(&media_type);
      cleanup();
      throw std::runtime_error("Failed to set subtype");
    }

    hr = MFSetAttributeSize(media_type, MF_MT_FRAME_SIZE, width, height);
    if (FAILED(hr)) {
      SafeRelease(&media_type);
      cleanup();
      throw std::runtime_error("Failed to set frame size");
    }

    hr = MFSetAttributeRatio(media_type, MF_MT_FRAME_RATE, fps, 1);
    if (FAILED(hr)) {
      SafeRelease(&media_type);
      cleanup();
      throw std::runtime_error("Failed to set frame rate");
    }

    hr = source_reader_->SetCurrentMediaType(
        MF_SOURCE_READER_FIRST_VIDEO_STREAM, nullptr, media_type);
    SafeRelease(&media_type);
    if (FAILED(hr)) {
      cleanup();
      throw std::runtime_error("Failed to set media type on source reader");
    }

    // 実際に設定されたメディアタイプを取得
    IMFMediaType* current_type = nullptr;
    hr = source_reader_->GetCurrentMediaType(
        MF_SOURCE_READER_FIRST_VIDEO_STREAM, &current_type);
    if (SUCCEEDED(hr)) {
      UINT32 w, h;
      MFGetAttributeSize(current_type, MF_MT_FRAME_SIZE, &w, &h);
      width_ = w;
      height_ = h;

      // ストライドを取得
      UINT32 default_stride = 0;
      hr = current_type->GetUINT32(MF_MT_DEFAULT_STRIDE, &default_stride);
      if (SUCCEEDED(hr)) {
        stride_ = default_stride;
      } else {
        // 取得できない場合はフォーマットから計算
        if (capture_format == Format::NV12) {
          stride_ = width_;
        } else if (capture_format == Format::YUY2 ||
                   capture_format == Format::UYVY) {
          stride_ = width_ * 2;
        }
      }
      SafeRelease(&current_type);
    } else {
      width_ = width;
      height_ = height;
      // デフォルトストライド
      if (capture_format == Format::NV12) {
        stride_ = width_;
      } else if (capture_format == Format::YUY2 ||
                 capture_format == Format::UYVY) {
        stride_ = width_ * 2;
      }
    }

    format_ = capture_format;
    running_ = true;

    // キャプチャスレッド開始
    capture_thread_ = std::thread(&DeviceWindows::capture_loop, this);
  }

  void stop() override {
    if (!running_)
      return;

    running_ = false;

    if (capture_thread_.joinable()) {
      capture_thread_.join();
    }

    cleanup();

    std::lock_guard<std::mutex> lock(frame_mutex_);
    latest_frame_ = nullptr;
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

    if (!activate_)
      return formats;

    IMFMediaSource* source = nullptr;
    HRESULT hr = activate_->ActivateObject(IID_PPV_ARGS(&source));
    if (FAILED(hr))
      return formats;

    IMFPresentationDescriptor* pd = nullptr;
    hr = source->CreatePresentationDescriptor(&pd);
    if (FAILED(hr)) {
      SafeRelease(&source);
      return formats;
    }

    DWORD stream_count = 0;
    pd->GetStreamDescriptorCount(&stream_count);

    for (DWORD i = 0; i < stream_count; i++) {
      BOOL selected = FALSE;
      IMFStreamDescriptor* sd = nullptr;
      hr = pd->GetStreamDescriptorByIndex(i, &selected, &sd);
      if (FAILED(hr))
        continue;

      IMFMediaTypeHandler* handler = nullptr;
      hr = sd->GetMediaTypeHandler(&handler);
      if (FAILED(hr)) {
        SafeRelease(&sd);
        continue;
      }

      DWORD type_count = 0;
      handler->GetMediaTypeCount(&type_count);

      for (DWORD j = 0; j < type_count; j++) {
        IMFMediaType* type = nullptr;
        hr = handler->GetMediaTypeByIndex(j, &type);
        if (FAILED(hr))
          continue;

        GUID subtype;
        hr = type->GetGUID(MF_MT_SUBTYPE, &subtype);
        if (FAILED(hr)) {
          SafeRelease(&type);
          continue;
        }

        // NV12, YUY2, UYVY のみ (MJPEG は非対応)
        Format fmt;
        if (subtype == MFVideoFormat_NV12) {
          fmt = Format::NV12;
        } else if (subtype == MFVideoFormat_YUY2) {
          fmt = Format::YUY2;
        } else if (subtype == MFVideoFormat_UYVY) {
          fmt = Format::UYVY;
        } else {
          SafeRelease(&type);
          continue;
        }

        UINT32 w, h;
        hr = MFGetAttributeSize(type, MF_MT_FRAME_SIZE, &w, &h);
        if (FAILED(hr)) {
          SafeRelease(&type);
          continue;
        }

        UINT32 num, denom;
        hr = MFGetAttributeRatio(type, MF_MT_FRAME_RATE, &num, &denom);
        if (FAILED(hr)) {
          num = 30;
          denom = 1;
        }

        FormatInfo info;
        info.width = w;
        info.height = h;
        info.fps = (denom > 0) ? (num / denom) : 30;
        info.format = fmt;
        formats.push_back(info);

        SafeRelease(&type);
      }

      SafeRelease(&handler);
      SafeRelease(&sd);
    }

    SafeRelease(&pd);
    source->Shutdown();
    SafeRelease(&source);

    return formats;
  }

  // 将来対応予定: 現在はスタブ実装
  void set_on_connected(DeviceCallback) override {}
  void set_on_disconnected(DeviceCallback) override {}

 private:
  void capture_loop() {
    while (running_) {
      DWORD stream_index, flags;
      LONGLONG timestamp;
      IMFSample* sample = nullptr;

      HRESULT hr = source_reader_->ReadSample(
          MF_SOURCE_READER_FIRST_VIDEO_STREAM, 0, &stream_index, &flags,
          &timestamp, &sample);

      if (FAILED(hr) || !sample) {
        SafeRelease(&sample);
        continue;
      }

      IMFMediaBuffer* buffer = nullptr;
      hr = sample->ConvertToContiguousBuffer(&buffer);
      if (FAILED(hr)) {
        SafeRelease(&sample);
        continue;
      }

      BYTE* data = nullptr;
      DWORD length = 0;
      hr = buffer->Lock(&data, nullptr, &length);
      if (FAILED(hr)) {
        SafeRelease(&buffer);
        SafeRelease(&sample);
        continue;
      }

      // フレームを作成
      std::shared_ptr<Frame> frame;

      if (format_ == Format::NV12) {
        size_t y_size = stride_ * height_;
        size_t uv_size = stride_ * height_ / 2;
        size_t total_size = y_size + uv_size;

        if (length >= total_size) {
          frame = std::make_shared<Frame>(width_, height_, Format::NV12);

          // バッファをコピーして use-after-free を防ぐ
          auto* buffer_copy = new std::vector<uint8_t>(length);
          std::memcpy(buffer_copy->data(), data, length);

          // Frame デストラクタでバッファを解放
          frame->set_native_buffer(buffer_copy, [](void* p) {
            delete static_cast<std::vector<uint8_t>*>(p);
          });

          // コピーしたデータへのポインタを設定
          frame->set_nv12_planes(buffer_copy->data(), stride_,
                                 buffer_copy->data() + y_size, stride_);
        }
      } else if (format_ == Format::YUY2) {
        size_t expected_size = stride_ * height_;

        if (length >= expected_size) {
          frame = std::make_shared<Frame>(width_, height_, Format::YUY2);

          // バッファをコピーして use-after-free を防ぐ
          auto* buffer_copy = new std::vector<uint8_t>(length);
          std::memcpy(buffer_copy->data(), data, length);

          // Frame デストラクタでバッファを解放
          frame->set_native_buffer(buffer_copy, [](void* p) {
            delete static_cast<std::vector<uint8_t>*>(p);
          });

          // コピーしたデータへのポインタを設定
          frame->set_packed_plane(buffer_copy->data(), stride_);
        }
      } else if (format_ == Format::UYVY) {
        size_t expected_size = stride_ * height_;

        if (length >= expected_size) {
          frame = std::make_shared<Frame>(width_, height_, Format::UYVY);

          // バッファをコピーして use-after-free を防ぐ
          auto* buffer_copy = new std::vector<uint8_t>(length);
          std::memcpy(buffer_copy->data(), data, length);

          // Frame デストラクタでバッファを解放
          frame->set_native_buffer(buffer_copy, [](void* p) {
            delete static_cast<std::vector<uint8_t>*>(p);
          });

          // コピーしたデータへのポインタを設定
          frame->set_packed_plane(buffer_copy->data(), stride_);
        }
      }

      buffer->Unlock();
      SafeRelease(&buffer);

      if (frame) {
        // タイムスタンプを設定 (100ns 単位 → マイクロ秒)
        frame->set_timestamp(static_cast<uint64_t>(timestamp / 10));

        std::lock_guard<std::mutex> lock(frame_mutex_);
        latest_frame_ = frame;
      }

      SafeRelease(&sample);
    }
  }

  void cleanup() {
    if (source_reader_) {
      SafeRelease(&source_reader_);
    }
    if (media_source_) {
      media_source_->Shutdown();
      SafeRelease(&media_source_);
    }
  }

  DeviceInfo info_;
  IMFActivate* activate_ = nullptr;
  IMFMediaSource* media_source_ = nullptr;
  IMFSourceReader* source_reader_ = nullptr;
  uint32_t width_ = 0;
  uint32_t height_ = 0;
  uint32_t stride_ = 0;
  Format format_ = Format::NV12;
  std::shared_ptr<Frame> latest_frame_;
  std::mutex frame_mutex_;
  std::thread capture_thread_;
  std::atomic<bool> running_{false};
};

// デバイス列挙
std::vector<DeviceInfo> list_devices_impl() {
  std::vector<DeviceInfo> devices;

  // Media Foundation を初期化
  if (!MFInitializer::instance().is_initialized()) {
    return devices;
  }

  IMFAttributes* attributes = nullptr;
  HRESULT hr = MFCreateAttributes(&attributes, 1);
  if (FAILED(hr))
    return devices;

  hr = attributes->SetGUID(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                           MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID);
  if (FAILED(hr)) {
    SafeRelease(&attributes);
    return devices;
  }

  IMFActivate** activates = nullptr;
  UINT32 count = 0;
  hr = MFEnumDeviceSources(attributes, &activates, &count);
  SafeRelease(&attributes);
  if (FAILED(hr))
    return devices;

  for (UINT32 i = 0; i < count; i++) {
    WCHAR* name = nullptr;
    UINT32 name_length = 0;
    hr = activates[i]->GetAllocatedString(MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
                                          &name, &name_length);
    if (SUCCEEDED(hr) && name) {
      DeviceInfo info;
      // WCHAR を UTF-8 に変換
      int size = WideCharToMultiByte(CP_UTF8, 0, name, -1, nullptr, 0, nullptr,
                                     nullptr);
      if (size > 0) {
        std::string str(size - 1, 0);
        WideCharToMultiByte(CP_UTF8, 0, name, -1, &str[0], size, nullptr,
                            nullptr);
        info.name = str;
      }
      CoTaskMemFree(name);

      // ユニーク ID を取得
      WCHAR* symbolic_link = nullptr;
      UINT32 link_length = 0;
      hr = activates[i]->GetAllocatedString(
          MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
          &symbolic_link, &link_length);
      if (SUCCEEDED(hr) && symbolic_link) {
        int size = WideCharToMultiByte(CP_UTF8, 0, symbolic_link, -1, nullptr,
                                       0, nullptr, nullptr);
        if (size > 0) {
          std::string str(size - 1, 0);
          WideCharToMultiByte(CP_UTF8, 0, symbolic_link, -1, &str[0], size,
                              nullptr, nullptr);
          info.unique_id = str;
        }
        CoTaskMemFree(symbolic_link);
      }

      info.index = static_cast<uint32_t>(devices.size());
      devices.push_back(info);
    }
    SafeRelease(&activates[i]);
  }
  CoTaskMemFree(activates);

  return devices;
}

// デバイスオープン
std::shared_ptr<Device> open_device_impl(uint32_t index) {
  // Media Foundation を初期化
  if (!MFInitializer::instance().is_initialized()) {
    throw std::runtime_error("Failed to initialize Media Foundation");
  }

  IMFAttributes* attributes = nullptr;
  HRESULT hr = MFCreateAttributes(&attributes, 1);
  if (FAILED(hr)) {
    throw std::runtime_error("Failed to create attributes");
  }

  hr = attributes->SetGUID(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                           MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID);
  if (FAILED(hr)) {
    SafeRelease(&attributes);
    throw std::runtime_error("Failed to set source type");
  }

  IMFActivate** activates = nullptr;
  UINT32 count = 0;
  hr = MFEnumDeviceSources(attributes, &activates, &count);
  SafeRelease(&attributes);
  if (FAILED(hr)) {
    throw std::runtime_error("Failed to enumerate devices");
  }

  if (index >= count) {
    for (UINT32 i = 0; i < count; i++) {
      SafeRelease(&activates[i]);
    }
    CoTaskMemFree(activates);
    throw std::runtime_error("Device index out of range");
  }

  // デバイス情報を取得
  DeviceInfo info;
  WCHAR* name = nullptr;
  UINT32 name_length = 0;
  hr = activates[index]->GetAllocatedString(
      MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, &name, &name_length);
  if (SUCCEEDED(hr) && name) {
    int size =
        WideCharToMultiByte(CP_UTF8, 0, name, -1, nullptr, 0, nullptr, nullptr);
    if (size > 0) {
      std::string str(size - 1, 0);
      WideCharToMultiByte(CP_UTF8, 0, name, -1, &str[0], size, nullptr,
                          nullptr);
      info.name = str;
    }
    CoTaskMemFree(name);
  }
  info.index = index;

  auto device = std::make_shared<DeviceWindows>(info, activates[index]);

  for (UINT32 i = 0; i < count; i++) {
    SafeRelease(&activates[i]);
  }
  CoTaskMemFree(activates);

  return device;
}

std::shared_ptr<Device> open_device_impl(const DeviceInfo& info) {
  return open_device_impl(info.index);
}

}  // namespace uvc
