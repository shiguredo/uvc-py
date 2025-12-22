import pytest

import uvc


@pytest.mark.skipif(
    len(uvc.list_devices()) == 0,
    reason="カメラデバイスが接続されていない",
)
def test_open_with_on_disconnected_callback():
    """on_disconnected コールバックを指定してデバイスを開けることを確認する"""
    devices = uvc.list_devices()
    disconnected_called = False

    def on_disconnected():
        nonlocal disconnected_called
        disconnected_called = True

    with uvc.open(devices[0], on_disconnected=on_disconnected) as device:
        assert device is not None
        assert device.info.name == devices[0].name


@pytest.mark.skipif(
    len(uvc.list_devices()) == 0,
    reason="カメラデバイスが接続されていない",
)
def test_open_with_on_connected_callback():
    """on_connected コールバックを指定してデバイスを開けることを確認する"""
    devices = uvc.list_devices()
    connected_called = False

    def on_connected():
        nonlocal connected_called
        connected_called = True

    with uvc.open(devices[0], on_connected=on_connected) as device:
        assert device is not None
        assert device.info.name == devices[0].name


@pytest.mark.skipif(
    len(uvc.list_devices()) == 0,
    reason="カメラデバイスが接続されていない",
)
def test_open_with_both_callbacks():
    """on_connected と on_disconnected 両方のコールバックを指定できることを確認する"""
    devices = uvc.list_devices()

    def on_connected():
        pass

    def on_disconnected():
        pass

    with uvc.open(
        devices[0],
        on_connected=on_connected,
        on_disconnected=on_disconnected,
    ) as device:
        assert device is not None
        assert device.info.name == devices[0].name


@pytest.mark.skipif(
    len(uvc.list_devices()) == 0,
    reason="カメラデバイスが接続されていない",
)
def test_open_without_callbacks():
    """コールバックなしでもデバイスを開けることを確認する"""
    devices = uvc.list_devices()

    with uvc.open(devices[0]) as device:
        assert device is not None
        assert device.info.name == devices[0].name


@pytest.mark.skipif(
    len(uvc.list_devices()) == 0,
    reason="カメラデバイスが接続されていない",
)
def test_open_with_none_callbacks():
    """None を渡してもデバイスを開けることを確認する"""
    devices = uvc.list_devices()

    with uvc.open(
        devices[0],
        on_connected=None,
        on_disconnected=None,
    ) as device:
        assert device is not None
        assert device.info.name == devices[0].name


@pytest.mark.skipif(
    len(uvc.list_devices()) == 0,
    reason="カメラデバイスが接続されていない",
)
def test_open_by_index_with_callbacks():
    """インデックス指定でコールバック付きでデバイスを開けることを確認する"""
    devices = uvc.list_devices()

    def on_disconnected():
        pass

    with uvc.open(0, on_disconnected=on_disconnected) as device:
        assert device is not None
        assert device.info.index == 0
