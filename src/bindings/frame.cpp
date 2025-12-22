#include "uvc.h"

#ifdef __APPLE__
#include <CoreVideo/CoreVideo.h>
#endif

namespace uvc {

Frame::Frame(uint32_t width, uint32_t height, Format format)
    : width_(width), height_(height), format_(format) {
  // ゼロコピーモードの場合は data_ を確保しない
  // NV12: set_nv12_planes() で直接ポインタをセット
  // YUY2/RGBA: set_packed_plane() で直接ポインタをセット
  if (format == Format::NV12 || format == Format::YUY2 ||
      format == Format::RGBA) {
    return;
  }

  size_t channels = 0;
  switch (format) {
    case Format::RGB:
      channels = 3;
      break;
    case Format::RGBA:
      channels = 4;
      break;
    case Format::MJPEG:
      channels = 3;
      break;
    default:
      break;
  }
  if (channels > 0) {
    data_.resize(width * height * channels);
  }
}

Frame::~Frame() {
  if (native_buffer_ && native_buffer_release_) {
    native_buffer_release_(native_buffer_);
  }
}

void Frame::set_native_buffer(void* buffer, void (*release_func)(void*)) {
  native_buffer_ = buffer;
  native_buffer_release_ = release_func;
}

void Frame::set_nv12_planes(uint8_t* y_plane,
                            size_t y_stride,
                            uint8_t* uv_plane,
                            size_t uv_stride) {
  y_plane_ = y_plane;
  y_stride_ = y_stride;
  uv_plane_ = uv_plane;
  uv_stride_ = uv_stride;
}

void Frame::set_packed_plane(uint8_t* plane, size_t stride) {
  packed_plane_ = plane;
  packed_stride_ = stride;
}

std::tuple<nb::ndarray<nb::numpy, uint8_t, nb::shape<-1, -1>>,
           nb::ndarray<nb::numpy, uint8_t, nb::shape<-1, -1>>>
Frame::to_nv12() const {
  if (format_ != Format::NV12) {
    throw std::runtime_error("Frame is not NV12 format");
  }

  if (!y_plane_ || !uv_plane_) {
    throw std::runtime_error("NV12 planes not set");
  }

  size_t y_shape[2] = {height_, width_};
  int64_t y_strides[2] = {static_cast<int64_t>(y_stride_), 1};
  auto y_array = nb::ndarray<nb::numpy, uint8_t, nb::shape<-1, -1>>(
      y_plane_, 2, y_shape, nb::handle(), y_strides);

  size_t uv_shape[2] = {height_ / 2, width_};
  int64_t uv_strides[2] = {static_cast<int64_t>(uv_stride_), 1};
  auto uv_array = nb::ndarray<nb::numpy, uint8_t, nb::shape<-1, -1>>(
      uv_plane_, 2, uv_shape, nb::handle(), uv_strides);

  return std::make_tuple(y_array, uv_array);
}

nb::ndarray<nb::numpy, uint8_t, nb::shape<-1, -1, -1>> Frame::to_yuy2() const {
  if (format_ != Format::YUY2) {
    throw std::runtime_error("Frame is not YUY2 format");
  }

  if (!packed_plane_) {
    throw std::runtime_error("Packed plane not set");
  }

  size_t shape[3] = {height_, width_, 2};
  int64_t strides[3] = {static_cast<int64_t>(packed_stride_), 2, 1};
  return nb::ndarray<nb::numpy, uint8_t, nb::shape<-1, -1, -1>>(
      packed_plane_, 3, shape, nb::handle(), strides);
}

nb::ndarray<nb::numpy, uint8_t, nb::shape<-1, -1, -1>> Frame::to_rgb() const {
  if (format_ != Format::RGB) {
    throw std::runtime_error("Frame is not RGB format");
  }

  if (data_.empty()) {
    throw std::runtime_error("RGB data not set");
  }

  size_t shape[3] = {height_, width_, 3};
  return nb::ndarray<nb::numpy, uint8_t, nb::shape<-1, -1, -1>>(
      const_cast<uint8_t*>(data_.data()), 3, shape);
}

nb::ndarray<nb::numpy, uint8_t, nb::shape<-1, -1, -1>> Frame::to_rgba() const {
  if (format_ != Format::RGBA) {
    throw std::runtime_error("Frame is not RGBA format");
  }

  if (!packed_plane_) {
    throw std::runtime_error("Packed plane not set");
  }

  size_t shape[3] = {height_, width_, 4};
  int64_t strides[3] = {static_cast<int64_t>(packed_stride_), 4, 1};
  return nb::ndarray<nb::numpy, uint8_t, nb::shape<-1, -1, -1>>(
      packed_plane_, 3, shape, nb::handle(), strides);
}

nb::object Frame::native_buffer() const {
#ifdef __APPLE__
  if (native_buffer_) {
    CVPixelBufferRetain(static_cast<CVPixelBufferRef>(native_buffer_));
    return nb::capsule(native_buffer_, "CVPixelBufferRef",
                       [](void* p) noexcept {
                         CVPixelBufferRelease(static_cast<CVPixelBufferRef>(p));
                       });
  }
#endif
  return nb::none();
}

}  // namespace uvc
