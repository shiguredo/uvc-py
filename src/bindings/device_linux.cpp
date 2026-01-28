#include "uvc.h"

#include <dirent.h>
#include <fcntl.h>
#include <linux/videodev2.h>
#include <sys/inotify.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <unistd.h>

#include <cstring>
#include <mutex>
#include <queue>
#include <thread>
#include <vector>

namespace uvc {

// V4L2 バッファ
struct V4L2Buffer {
  void* start;
  size_t length;
};

// Linux デバイス実装
class DeviceLinux : public Device {
 public:
  DeviceLinux(const DeviceInfo& info, const std::string& device_path)
      : info_(info), device_path_(device_path) {}

  ~DeviceLinux() override { stop(); }

  void start(uint32_t width,
             uint32_t height,
             uint32_t fps,
             Format capture_format,
             std::optional<Format> output_format_opt) override {
    if (running_)
      return;

    // Linux では MJPEG 非対応
    if (capture_format == Format::MJPEG) {
      throw std::runtime_error(
          "MJPEG is not supported on Linux. Use NV12 or YUY2.");
    }

    // output_format は Linux では無視 (capture_format と同じになる)
    (void)output_format_opt;

    // デバイスをオープン
    fd_ = ::open(device_path_.c_str(), O_RDWR | O_NONBLOCK);
    if (fd_ < 0) {
      throw std::runtime_error("Failed to open device: " + device_path_);
    }

    // V4L2 ピクセルフォーマットを決定
    uint32_t v4l2_pixfmt;
    switch (capture_format) {
      case Format::NV12:
        v4l2_pixfmt = V4L2_PIX_FMT_NV12;
        break;
      case Format::YUY2:
        v4l2_pixfmt = V4L2_PIX_FMT_YUYV;
        break;
      default:
        ::close(fd_);
        fd_ = -1;
        throw std::runtime_error("Unsupported format. Use NV12 or YUY2.");
    }

    // フォーマット設定
    struct v4l2_format fmt = {};
    fmt.type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
    fmt.fmt.pix.width = width;
    fmt.fmt.pix.height = height;
    fmt.fmt.pix.pixelformat = v4l2_pixfmt;
    fmt.fmt.pix.field = V4L2_FIELD_NONE;

    if (ioctl(fd_, VIDIOC_S_FMT, &fmt) < 0) {
      ::close(fd_);
      fd_ = -1;
      throw std::runtime_error("Failed to set format");
    }

    width_ = fmt.fmt.pix.width;
    height_ = fmt.fmt.pix.height;
    stride_ = fmt.fmt.pix.bytesperline;
    format_ = capture_format;

    // フレームレート設定
    struct v4l2_streamparm parm = {};
    parm.type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
    parm.parm.capture.timeperframe.numerator = 1;
    parm.parm.capture.timeperframe.denominator = fps;
    ioctl(fd_, VIDIOC_S_PARM, &parm);

    // バッファ要求
    struct v4l2_requestbuffers req = {};
    req.count = 4;
    req.type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
    req.memory = V4L2_MEMORY_MMAP;

    if (ioctl(fd_, VIDIOC_REQBUFS, &req) < 0) {
      ::close(fd_);
      fd_ = -1;
      throw std::runtime_error("Failed to request buffers");
    }

    // バッファをマップ
    buffers_.resize(req.count);
    for (uint32_t i = 0; i < req.count; i++) {
      struct v4l2_buffer buf = {};
      buf.type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
      buf.memory = V4L2_MEMORY_MMAP;
      buf.index = i;

      if (ioctl(fd_, VIDIOC_QUERYBUF, &buf) < 0) {
        cleanup_buffers();
        ::close(fd_);
        fd_ = -1;
        throw std::runtime_error("Failed to query buffer");
      }

      buffers_[i].length = buf.length;
      buffers_[i].start = mmap(nullptr, buf.length, PROT_READ | PROT_WRITE,
                               MAP_SHARED, fd_, buf.m.offset);

      if (buffers_[i].start == MAP_FAILED) {
        cleanup_buffers();
        ::close(fd_);
        fd_ = -1;
        throw std::runtime_error("Failed to mmap buffer");
      }
    }

    // バッファをキューに入れる
    for (uint32_t i = 0; i < buffers_.size(); i++) {
      struct v4l2_buffer buf = {};
      buf.type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
      buf.memory = V4L2_MEMORY_MMAP;
      buf.index = i;

      if (ioctl(fd_, VIDIOC_QBUF, &buf) < 0) {
        cleanup_buffers();
        ::close(fd_);
        fd_ = -1;
        throw std::runtime_error("Failed to queue buffer");
      }
    }

    // ストリーム開始
    enum v4l2_buf_type type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
    if (ioctl(fd_, VIDIOC_STREAMON, &type) < 0) {
      cleanup_buffers();
      ::close(fd_);
      fd_ = -1;
      throw std::runtime_error("Failed to start stream");
    }

    running_ = true;

    // inotify でデバイスの接続・切断を監視
    inotify_fd_ = inotify_init1(IN_NONBLOCK);
    if (inotify_fd_ >= 0) {
      inotify_wd_ = inotify_add_watch(inotify_fd_, "/sys/class/video4linux",
                                      IN_CREATE | IN_DELETE);
    }

    // キャプチャスレッド開始
    capture_thread_ = std::thread(&DeviceLinux::capture_loop, this);
  }

  void stop() override {
    if (!running_)
      return;

    running_ = false;

    if (capture_thread_.joinable()) {
      capture_thread_.join();
    }

    // inotify クリーンアップ
    if (inotify_fd_ >= 0) {
      if (inotify_wd_ >= 0) {
        inotify_rm_watch(inotify_fd_, inotify_wd_);
        inotify_wd_ = -1;
      }
      ::close(inotify_fd_);
      inotify_fd_ = -1;
    }

    if (fd_ >= 0) {
      enum v4l2_buf_type type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
      ioctl(fd_, VIDIOC_STREAMOFF, &type);
      cleanup_buffers();
      ::close(fd_);
      fd_ = -1;
    }

    std::lock_guard<std::mutex> lock(queue_mutex_);
    while (!frame_queue_.empty()) {
      frame_queue_.pop();
    }
  }

  std::shared_ptr<Frame> get_frame() override {
    std::lock_guard<std::mutex> lock(queue_mutex_);
    if (frame_queue_.empty()) {
      return nullptr;
    }
    auto frame = frame_queue_.front();
    frame_queue_.pop();
    return frame;
  }

  bool is_running() const override { return running_; }

  const DeviceInfo& info() const override { return info_; }

  std::vector<FormatInfo> get_supported_formats() const override {
    std::vector<FormatInfo> formats;

    int fd = ::open(device_path_.c_str(), O_RDWR);
    if (fd < 0)
      return formats;

    // 各ピクセルフォーマットを列挙
    // MJPEG は非対応のためリストから除外
    uint32_t pix_formats[] = {V4L2_PIX_FMT_NV12, V4L2_PIX_FMT_YUYV};
    Format fmt_types[] = {Format::NV12, Format::YUY2};

    for (size_t i = 0; i < 2; i++) {
      struct v4l2_frmsizeenum frmsize = {};
      frmsize.pixel_format = pix_formats[i];

      while (ioctl(fd, VIDIOC_ENUM_FRAMESIZES, &frmsize) == 0) {
        if (frmsize.type == V4L2_FRMSIZE_TYPE_DISCRETE) {
          struct v4l2_frmivalenum ivalenum = {};
          ivalenum.pixel_format = pix_formats[i];
          ivalenum.width = frmsize.discrete.width;
          ivalenum.height = frmsize.discrete.height;

          while (ioctl(fd, VIDIOC_ENUM_FRAMEINTERVALS, &ivalenum) == 0) {
            if (ivalenum.type == V4L2_FRMIVAL_TYPE_DISCRETE) {
              FormatInfo info;
              info.width = frmsize.discrete.width;
              info.height = frmsize.discrete.height;
              info.fps =
                  ivalenum.discrete.denominator / ivalenum.discrete.numerator;
              info.format = fmt_types[i];
              formats.push_back(info);
            }
            ivalenum.index++;
          }
        }
        frmsize.index++;
      }
    }

    ::close(fd);
    return formats;
  }

  void set_on_connected(DeviceCallback callback) override {
    on_connected_ = std::move(callback);
  }

  void set_on_disconnected(DeviceCallback callback) override {
    on_disconnected_ = std::move(callback);
  }

 private:
  void capture_loop() {
    std::string device_name = get_device_name();

    while (running_) {
      fd_set fds;
      FD_ZERO(&fds);
      FD_SET(fd_, &fds);

      int max_fd = fd_;
      if (inotify_fd_ >= 0) {
        FD_SET(inotify_fd_, &fds);
        if (inotify_fd_ > max_fd) {
          max_fd = inotify_fd_;
        }
      }

      struct timeval tv = {0, 100000};

      int r = select(max_fd + 1, &fds, nullptr, nullptr, &tv);
      if (r <= 0)
        continue;

      // inotify イベントを処理
      if (inotify_fd_ >= 0 && FD_ISSET(inotify_fd_, &fds)) {
        process_inotify_events(device_name);
      }

      // V4L2 フレームを処理
      if (!FD_ISSET(fd_, &fds)) {
        continue;
      }

      struct v4l2_buffer buf = {};
      buf.type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
      buf.memory = V4L2_MEMORY_MMAP;

      if (ioctl(fd_, VIDIOC_DQBUF, &buf) < 0) {
        continue;
      }

      // フレームを作成
      const uint8_t* data =
          static_cast<const uint8_t*>(buffers_[buf.index].start);
      size_t size = buf.bytesused;

      std::shared_ptr<Frame> frame;

      if (format_ == Format::NV12) {
        // NV12: Y プレーン (stride * height) + UV プレーン (stride * height / 2)
        size_t y_size = stride_ * height_;
        size_t uv_size = stride_ * height_ / 2;
        size_t total_size = y_size + uv_size;

        if (size >= total_size) {
          frame = std::make_shared<Frame>(width_, height_, Format::NV12);

          // MMAP バッファからデータをコピー
          auto* buffer = new std::vector<uint8_t>(total_size);
          std::memcpy(buffer->data(), data, total_size);

          // Frame デストラクタでバッファを解放
          frame->set_native_buffer(buffer, [](void* p) {
            delete static_cast<std::vector<uint8_t>*>(p);
          });

          // コピーしたデータへのポインタを設定
          frame->set_nv12_planes(buffer->data(), stride_,
                                 buffer->data() + y_size, stride_);
        }
      } else if (format_ == Format::YUY2) {
        // YUY2: packed format (stride * height bytes)
        size_t expected_size = stride_ * height_;

        if (size >= expected_size) {
          frame = std::make_shared<Frame>(width_, height_, Format::YUY2);

          // MMAP バッファからデータをコピー
          auto* buffer = new std::vector<uint8_t>(expected_size);
          std::memcpy(buffer->data(), data, expected_size);

          // Frame デストラクタでバッファを解放
          frame->set_native_buffer(buffer, [](void* p) {
            delete static_cast<std::vector<uint8_t>*>(p);
          });

          // コピーしたデータへのポインタを設定
          frame->set_packed_plane(buffer->data(), stride_);
        }
      }

      if (frame) {
        frame->set_timestamp(buf.timestamp.tv_sec * 1000000 +
                             buf.timestamp.tv_usec);

        std::lock_guard<std::mutex> lock(queue_mutex_);
        // 最新フレームのみ保持
        while (frame_queue_.size() >= 1) {
          frame_queue_.pop();
        }
        frame_queue_.push(frame);
      }

      // バッファを再キュー
      if (ioctl(fd_, VIDIOC_QBUF, &buf) < 0) {
        // エラー処理
      }
    }
  }

  void process_inotify_events(const std::string& device_name) {
    char buffer[4096];
    ssize_t len = read(inotify_fd_, buffer, sizeof(buffer));
    if (len <= 0) {
      return;
    }

    char* ptr = buffer;
    while (ptr < buffer + len) {
      auto* event = reinterpret_cast<struct inotify_event*>(ptr);

      if (event->len > 0) {
        std::string name(event->name);
        if (name == device_name) {
          if (event->mask & IN_CREATE) {
            if (on_connected_) {
              on_connected_();
            }
          } else if (event->mask & IN_DELETE) {
            if (on_disconnected_) {
              on_disconnected_();
            }
          }
        }
      }

      ptr += sizeof(struct inotify_event) + event->len;
    }
  }

  void cleanup_buffers() {
    for (auto& buffer : buffers_) {
      if (buffer.start && buffer.start != MAP_FAILED) {
        munmap(buffer.start, buffer.length);
      }
    }
    buffers_.clear();
  }

  // デバイスパスからデバイス名を抽出
  // 例: "/dev/video0" -> "video0"
  std::string get_device_name() const {
    size_t pos = device_path_.rfind('/');
    if (pos != std::string::npos) {
      return device_path_.substr(pos + 1);
    }
    return device_path_;
  }

  DeviceInfo info_;
  std::string device_path_;
  int fd_ = -1;
  uint32_t width_ = 0;
  uint32_t height_ = 0;
  uint32_t stride_ = 0;
  Format format_ = Format::NV12;
  std::vector<V4L2Buffer> buffers_;
  std::queue<std::shared_ptr<Frame>> frame_queue_;
  std::mutex queue_mutex_;
  std::thread capture_thread_;
  bool running_ = false;

  // inotify 関連
  int inotify_fd_ = -1;
  int inotify_wd_ = -1;
  DeviceCallback on_connected_;
  DeviceCallback on_disconnected_;
};

// デバイス列挙
std::vector<DeviceInfo> list_devices_impl() {
  std::vector<DeviceInfo> devices;

  DIR* dir = opendir("/dev");
  if (!dir)
    return devices;

  struct dirent* entry;
  uint32_t index = 0;

  while ((entry = readdir(dir)) != nullptr) {
    if (strncmp(entry->d_name, "video", 5) != 0)
      continue;

    std::string path = std::string("/dev/") + entry->d_name;
    int fd = ::open(path.c_str(), O_RDWR);
    if (fd < 0)
      continue;

    struct v4l2_capability cap = {};
    if (ioctl(fd, VIDIOC_QUERYCAP, &cap) == 0) {
      if (cap.device_caps & V4L2_CAP_VIDEO_CAPTURE) {
        DeviceInfo info;
        info.name = reinterpret_cast<const char*>(cap.card);
        info.unique_id = path;
        info.index = index++;
        devices.push_back(info);
      }
    }

    ::close(fd);
  }

  closedir(dir);
  return devices;
}

// デバイスオープン
std::shared_ptr<Device> open_device_impl(uint32_t index) {
  auto devices = list_devices_impl();
  if (index >= devices.size()) {
    throw std::runtime_error("Device index out of range");
  }
  return std::make_shared<DeviceLinux>(devices[index],
                                       devices[index].unique_id);
}

std::shared_ptr<Device> open_device_impl(const DeviceInfo& info) {
  return std::make_shared<DeviceLinux>(info, info.unique_id);
}

}  // namespace uvc
