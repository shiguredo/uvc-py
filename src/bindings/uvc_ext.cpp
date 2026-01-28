#include "uvc.h"

using namespace nb::literals;

NB_MODULE(uvc_ext, m) {
  m.doc() = "UVC camera library with hardware JPEG decoding";

  // Format enum
  nb::enum_<uvc::Format>(m, "Format")
      .value("MJPEG", uvc::Format::MJPEG)
      .value("YUY2", uvc::Format::YUY2)
      .value("UYVY", uvc::Format::UYVY)
      .value("NV12", uvc::Format::NV12)
      .value("RGB", uvc::Format::RGB)
      .value("RGBA", uvc::Format::RGBA)
      .value("BGRA", uvc::Format::BGRA);

  // DeviceInfo
  nb::class_<uvc::DeviceInfo>(m, "DeviceInfo")
      .def_ro("name", &uvc::DeviceInfo::name)
      .def_ro("unique_id", &uvc::DeviceInfo::unique_id)
      .def_ro("index", &uvc::DeviceInfo::index)
      .def("__repr__", [](const uvc::DeviceInfo& info) {
        return "DeviceInfo(name='" + info.name +
               "', index=" + std::to_string(info.index) + ")";
      });

  // FormatInfo
  nb::class_<uvc::FormatInfo>(m, "FormatInfo")
      .def_ro("width", &uvc::FormatInfo::width)
      .def_ro("height", &uvc::FormatInfo::height)
      .def_ro("fps", &uvc::FormatInfo::fps)
      .def_ro("format", &uvc::FormatInfo::format)
      .def("__repr__", [](const uvc::FormatInfo& info) {
        std::string fmt_str;
        switch (info.format) {
          case uvc::Format::MJPEG:
            fmt_str = "MJPEG";
            break;
          case uvc::Format::YUY2:
            fmt_str = "YUY2";
            break;
          case uvc::Format::UYVY:
            fmt_str = "UYVY";
            break;
          case uvc::Format::NV12:
            fmt_str = "NV12";
            break;
          case uvc::Format::RGB:
            fmt_str = "RGB";
            break;
          case uvc::Format::RGBA:
            fmt_str = "RGBA";
            break;
          case uvc::Format::BGRA:
            fmt_str = "BGRA";
            break;
        }
        return std::to_string(info.width) + "x" + std::to_string(info.height) +
               "@" + std::to_string(info.fps) + "fps (" + fmt_str + ")";
      });

  // Frame
  nb::class_<uvc::Frame>(m, "Frame")
      .def_prop_ro("width", &uvc::Frame::width)
      .def_prop_ro("height", &uvc::Frame::height)
      .def_prop_ro("format", &uvc::Frame::format)
      .def_prop_ro("timestamp", &uvc::Frame::timestamp)
      .def(
          "to_nv12",
          [](std::shared_ptr<uvc::Frame> self) {
            auto [y, uv] = self->to_nv12();
            nb::object owner = nb::cast(self);

            size_t y_shape[2] = {static_cast<size_t>(y.shape(0)),
                                 static_cast<size_t>(y.shape(1))};
            int64_t y_strides[2] = {y.stride(0), y.stride(1)};
            auto y_arr = nb::ndarray<nb::numpy, uint8_t, nb::shape<-1, -1>>(
                y.data(), 2, y_shape, owner, y_strides);

            size_t uv_shape[2] = {static_cast<size_t>(uv.shape(0)),
                                  static_cast<size_t>(uv.shape(1))};
            int64_t uv_strides[2] = {uv.stride(0), uv.stride(1)};
            auto uv_arr = nb::ndarray<nb::numpy, uint8_t, nb::shape<-1, -1>>(
                uv.data(), 2, uv_shape, owner, uv_strides);

            return std::make_tuple(y_arr, uv_arr);
          },
          "Get NV12 Y and UV planes as (Y, UV) tuple")
      .def(
          "to_yuy2",
          [](std::shared_ptr<uvc::Frame> self) {
            auto result = self->to_yuy2();
            size_t shape[3] = {static_cast<size_t>(result.shape(0)),
                               static_cast<size_t>(result.shape(1)),
                               static_cast<size_t>(result.shape(2))};
            int64_t strides[3] = {result.stride(0), result.stride(1),
                                  result.stride(2)};
            return nb::ndarray<nb::numpy, uint8_t, nb::shape<-1, -1, -1>>(
                result.data(), 3, shape, nb::cast(self), strides);
          },
          "Get YUY2 data as (H, W, 2) array")
      .def(
          "to_uyvy",
          [](std::shared_ptr<uvc::Frame> self) {
            auto result = self->to_uyvy();
            size_t shape[3] = {static_cast<size_t>(result.shape(0)),
                               static_cast<size_t>(result.shape(1)),
                               static_cast<size_t>(result.shape(2))};
            int64_t strides[3] = {result.stride(0), result.stride(1),
                                  result.stride(2)};
            return nb::ndarray<nb::numpy, uint8_t, nb::shape<-1, -1, -1>>(
                result.data(), 3, shape, nb::cast(self), strides);
          },
          "Get UYVY data as (H, W, 2) array")
      .def(
          "to_rgb",
          [](std::shared_ptr<uvc::Frame> self) {
            auto result = self->to_rgb();
            size_t shape[3] = {static_cast<size_t>(result.shape(0)),
                               static_cast<size_t>(result.shape(1)),
                               static_cast<size_t>(result.shape(2))};
            return nb::ndarray<nb::numpy, uint8_t, nb::shape<-1, -1, -1>>(
                result.data(), 3, shape, nb::cast(self));
          },
          "Get RGB data as (H, W, 3) array")
      .def(
          "to_rgba",
          [](std::shared_ptr<uvc::Frame> self) {
            auto result = self->to_rgba();
            size_t shape[3] = {static_cast<size_t>(result.shape(0)),
                               static_cast<size_t>(result.shape(1)),
                               static_cast<size_t>(result.shape(2))};
            int64_t strides[3] = {result.stride(0), result.stride(1),
                                  result.stride(2)};
            return nb::ndarray<nb::numpy, uint8_t, nb::shape<-1, -1, -1>>(
                result.data(), 3, shape, nb::cast(self), strides);
          },
          "Get RGBA data as (H, W, 4) array")
      .def("native_buffer", &uvc::Frame::native_buffer,
           "Get native buffer as PyCapsule (macOS: CVPixelBufferRef, Linux: "
           "None)");

  // Device
  nb::class_<uvc::Device>(m, "Device")
      .def("start", &uvc::Device::start, "width"_a, "height"_a, "fps"_a,
           "capture_format"_a = uvc::Format::NV12,
           "output_format"_a = nb::none(), nb::lock_self(), "Start capturing")
      .def("stop", &uvc::Device::stop, nb::lock_self(), "Stop capturing")
      .def(
          "get_frame",
          [](uvc::Device& self) {
            std::shared_ptr<uvc::Frame> frame;
            {
              nb::gil_scoped_release release;
              frame = self.get_frame();
            }
            return frame;
          },
          nb::lock_self(), "Get next frame")
      .def_prop_ro("is_running", &uvc::Device::is_running)
      .def_prop_ro("info", &uvc::Device::info)
      .def("get_supported_formats", &uvc::Device::get_supported_formats,
           nb::lock_self(), "Get list of FormatInfo")
      .def(
          "__enter__", [](std::shared_ptr<uvc::Device> self) { return self; },
          "Enter context manager")
      .def(
          "__exit__",
          [](uvc::Device& self, nb::object, nb::object, nb::object) {
            if (self.is_running()) {
              self.stop();
            }
          },
          nb::arg().none(), nb::arg().none(), nb::arg().none(),
          "Exit context manager");

  // list_devices function
  m.def("list_devices", &uvc::list_devices_impl, "List available UVC devices");

  // open function (by index)
  m.def(
      "open",
      [](uint32_t index, std::optional<nb::object> on_connected,
         std::optional<nb::object> on_disconnected)
          -> std::shared_ptr<uvc::Device> {
        auto device = uvc::open_device_impl(index);

        if (on_connected.has_value() && !on_connected->is_none()) {
          nb::object callback = *on_connected;
          device->set_on_connected([callback]() {
            // コールバックは Python
            // 外のスレッドから呼ばれるのでスレッドステート取得が必要
            nb::gil_scoped_acquire gil;
            callback();
          });
        }

        if (on_disconnected.has_value() && !on_disconnected->is_none()) {
          nb::object callback = *on_disconnected;
          device->set_on_disconnected([callback]() {
            // コールバックは Python
            // 外のスレッドから呼ばれるのでスレッドステート取得が必要
            nb::gil_scoped_acquire gil;
            callback();
          });
        }

        return device;
      },
      "index"_a, "on_connected"_a = nb::none(),
      "on_disconnected"_a = nb::none(), "Open device by index");

  // open function (by DeviceInfo)
  m.def(
      "open",
      [](const uvc::DeviceInfo& info, std::optional<nb::object> on_connected,
         std::optional<nb::object> on_disconnected)
          -> std::shared_ptr<uvc::Device> {
        auto device = uvc::open_device_impl(info);

        if (on_connected.has_value() && !on_connected->is_none()) {
          nb::object callback = *on_connected;
          device->set_on_connected([callback]() {
            // コールバックは Python
            // 外のスレッドから呼ばれるのでスレッドステート取得が必要
            nb::gil_scoped_acquire gil;
            callback();
          });
        }

        if (on_disconnected.has_value() && !on_disconnected->is_none()) {
          nb::object callback = *on_disconnected;
          device->set_on_disconnected([callback]() {
            // コールバックは Python
            // 外のスレッドから呼ばれるのでスレッドステート取得が必要
            nb::gil_scoped_acquire gil;
            callback();
          });
        }

        return device;
      },
      "info"_a, "on_connected"_a = nb::none(), "on_disconnected"_a = nb::none(),
      "Open device by DeviceInfo");
}
