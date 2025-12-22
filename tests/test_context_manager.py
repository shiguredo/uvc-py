import time

import pytest

import uvc


@pytest.mark.skipif(
    len(uvc.list_devices()) == 0,
    reason="カメラデバイスが接続されていない",
)
def test_exit_does_not_suppress_exception():
    """__exit__ が例外を抑制しないことを確認する"""
    devices = uvc.list_devices()
    device = None

    with pytest.raises(ValueError, match="test exception"):
        with uvc.open(devices[0]) as device:
            formats = device.get_supported_formats()
            fmt = formats[0]
            device.start(fmt.width, fmt.height, fmt.fps, fmt.format)
            time.sleep(0.1)
            raise ValueError("test exception")

    # with ブロックを抜けた後、デバイスは停止しているはず
    assert device is not None
    assert not device.is_running


@pytest.mark.skipif(
    len(uvc.list_devices()) == 0,
    reason="カメラデバイスが接続されていない",
)
def test_context_manager_stops_on_exit():
    """context manager が終了時に stop を呼ぶことを確認する"""
    devices = uvc.list_devices()

    with uvc.open(devices[0]) as device:
        formats = device.get_supported_formats()
        fmt = formats[0]
        device.start(fmt.width, fmt.height, fmt.fps, fmt.format)
        time.sleep(0.1)
        assert device.is_running

    assert not device.is_running


@pytest.mark.skipif(
    len(uvc.list_devices()) == 0,
    reason="カメラデバイスが接続されていない",
)
def test_enter_returns_device():
    """__enter__ が device 自身を返すことを確認する"""
    devices = uvc.list_devices()
    device = uvc.open(devices[0])

    with device as ctx:
        assert ctx is device
