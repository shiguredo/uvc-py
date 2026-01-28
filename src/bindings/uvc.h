#pragma once

#include <nanobind/nanobind.h>
#include <nanobind/ndarray.h>
#include <nanobind/stl/optional.h>
#include <nanobind/stl/shared_ptr.h>
#include <nanobind/stl/string.h>
#include <nanobind/stl/tuple.h>
#include <nanobind/stl/vector.h>

#include <cstdint>
#include <functional>
#include <memory>
#include <optional>
#include <string>
#include <vector>

namespace nb = nanobind;

namespace uvc {

// デバイスコールバック型
using DeviceCallback = std::function<void()>;

// フォーマット
enum class Format {
  MJPEG,
  YUY2,
  UYVY,
  NV12,
  RGB,
  RGBA,
  BGRA,
};

// デバイス情報
struct DeviceInfo {
  std::string name;
  std::string unique_id;
  uint32_t index;
};

// フォーマット情報
struct FormatInfo {
  uint32_t width;
  uint32_t height;
  uint32_t fps;
  Format format;
};

// フレームデータ
class Frame {
 public:
  Frame(uint32_t width, uint32_t height, Format format);
  ~Frame();

  // コピー禁止
  Frame(const Frame&) = delete;
  Frame& operator=(const Frame&) = delete;

  uint32_t width() const { return width_; }
  uint32_t height() const { return height_; }
  Format format() const { return format_; }
  uint64_t timestamp() const { return timestamp_; }
  void set_timestamp(uint64_t ts) { timestamp_ = ts; }

  uint8_t* data() { return data_.data(); }
  const uint8_t* data() const { return data_.data(); }
  size_t size() const { return data_.size(); }

  // NV12 フォーマット: Y プレーンと UV プレーンを取得
  // Y: (H, W), UV: (H/2, W)
  std::tuple<nb::ndarray<nb::numpy, uint8_t, nb::shape<-1, -1>>,
             nb::ndarray<nb::numpy, uint8_t, nb::shape<-1, -1>>>
  to_nv12() const;

  // YUY2 フォーマット: (H, W, 2)
  nb::ndarray<nb::numpy, uint8_t, nb::shape<-1, -1, -1>> to_yuy2() const;

  // UYVY フォーマット: (H, W, 2)
  nb::ndarray<nb::numpy, uint8_t, nb::shape<-1, -1, -1>> to_uyvy() const;

  // RGB フォーマット: (H, W, 3)
  nb::ndarray<nb::numpy, uint8_t, nb::shape<-1, -1, -1>> to_rgb() const;

  // RGBA フォーマット: (H, W, 4)
  nb::ndarray<nb::numpy, uint8_t, nb::shape<-1, -1, -1>> to_rgba() const;

  // ネイティブバッファを PyCapsule として取得
  // macOS: CVPixelBufferRef を "CVPixelBufferRef" という名前の capsule で返す
  // Linux: None を返す
  nb::object native_buffer() const;

  // ゼロコピー用: プラットフォーム固有のバッファを保持
  void set_native_buffer(void* buffer, void (*release_func)(void*));
  void set_nv12_planes(uint8_t* y_plane,
                       size_t y_stride,
                       uint8_t* uv_plane,
                       size_t uv_stride);
  void set_packed_plane(uint8_t* plane, size_t stride);

 private:
  uint32_t width_;
  uint32_t height_;
  Format format_;
  uint64_t timestamp_ = 0;
  std::vector<uint8_t> data_;

  // ゼロコピー用
  void* native_buffer_ = nullptr;
  void (*native_buffer_release_)(void*) = nullptr;
  uint8_t* y_plane_ = nullptr;
  size_t y_stride_ = 0;
  uint8_t* uv_plane_ = nullptr;
  size_t uv_stride_ = 0;
  uint8_t* packed_plane_ = nullptr;
  size_t packed_stride_ = 0;
};

// デバイスの抽象クラス
class Device {
 public:
  virtual ~Device() = default;

  // capture_format: カメラからのキャプチャフォーマット
  // output_format: 出力フォーマット (省略時は capture_format と同じ)
  virtual void start(uint32_t width,
                     uint32_t height,
                     uint32_t fps,
                     Format capture_format = Format::NV12,
                     std::optional<Format> output_format = std::nullopt) = 0;
  virtual void stop() = 0;
  virtual std::shared_ptr<Frame> get_frame() = 0;

  virtual bool is_running() const = 0;
  virtual const DeviceInfo& info() const = 0;

  // サポートされているフォーマットを取得
  virtual std::vector<FormatInfo> get_supported_formats() const = 0;

  // コールバック設定
  virtual void set_on_connected(DeviceCallback callback) = 0;
  virtual void set_on_disconnected(DeviceCallback callback) = 0;
};

// デバイス列挙（プラットフォーム固有実装）
std::vector<DeviceInfo> list_devices_impl();

// デバイスオープン（プラットフォーム固有実装）
std::shared_ptr<Device> open_device_impl(uint32_t index);
std::shared_ptr<Device> open_device_impl(const DeviceInfo& info);

}  // namespace uvc
