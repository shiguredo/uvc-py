import platform
import time

import pytest

import uvc


@pytest.mark.skipif(
    len(uvc.list_devices()) == 0,
    reason="カメラデバイスが接続されていない",
)
def test_native_buffer_returns_capsule_on_macos():
    """macOS で native_buffer() が capsule を返すことを確認する"""
    devices = uvc.list_devices()

    with uvc.open(devices[0]) as device:
        formats = device.get_supported_formats()
        assert len(formats) > 0

        fmt = formats[0]
        device.start(fmt.width, fmt.height, fmt.fps, fmt.format)

        # カメラの起動を待つ
        time.sleep(0.5)

        frame = None
        for _ in range(100):
            frame = device.get_frame()
            if frame is not None:
                break
            time.sleep(0.05)

    assert frame is not None

    native = frame.native_buffer()

    if platform.system() == "Darwin":
        # macOS では capsule が返される
        assert native is not None
        # PyCapsule の名前を確認
        import ctypes

        pythonapi = ctypes.pythonapi
        pythonapi.PyCapsule_GetName.restype = ctypes.c_char_p
        pythonapi.PyCapsule_GetName.argtypes = [ctypes.py_object]
        name = pythonapi.PyCapsule_GetName(native)
        assert name == b"CVPixelBufferRef"
    else:
        # Linux では None が返される
        assert native is None


@pytest.mark.skipif(
    len(uvc.list_devices()) == 0,
    reason="カメラデバイスが接続されていない",
)
def test_native_buffer_multiple_calls():
    """native_buffer() を複数回呼び出しても問題ないことを確認する"""
    devices = uvc.list_devices()

    with uvc.open(devices[0]) as device:
        formats = device.get_supported_formats()
        fmt = formats[0]
        device.start(fmt.width, fmt.height, fmt.fps, fmt.format)

        # カメラの起動を待つ
        time.sleep(0.5)

        frame = None
        for _ in range(100):
            frame = device.get_frame()
            if frame is not None:
                break
            time.sleep(0.05)

    assert frame is not None

    # 複数回呼び出し
    native1 = frame.native_buffer()
    native2 = frame.native_buffer()

    if platform.system() == "Darwin":
        # 両方とも有効な capsule
        assert native1 is not None
        assert native2 is not None
