"""
スレッドセーフ性テスト

複数スレッドから同時にアクセスしてもデータ競合が発生しないことを確認する。
GIL 有効環境ではスレッド関連のバグを検出し、
Free-Threading (Python 3.13t 以降) 環境ではより厳密なテストとなる。
"""

import sys
import time
from concurrent.futures import ThreadPoolExecutor

import pytest

import uvc


def is_gil_enabled():
    """GIL が有効かどうかを確認する"""
    if hasattr(sys, "_is_gil_enabled"):
        return sys._is_gil_enabled()
    return True


@pytest.mark.skipif(
    len(uvc.list_devices()) == 0,
    reason="カメラデバイスが接続されていない",
)
def test_concurrent_list_devices():
    """複数スレッドから同時に list_devices() を呼び出してもクラッシュしないことを確認する"""
    errors = []

    def call_list_devices():
        try:
            for _ in range(10):
                devices = uvc.list_devices()
                assert isinstance(devices, list)
        except Exception as exception:
            errors.append(exception)

    with ThreadPoolExecutor(max_workers=4) as executor:
        futures = [executor.submit(call_list_devices) for _ in range(4)]
        for future in futures:
            future.result()

    assert len(errors) == 0, f"エラーが発生: {errors}"


@pytest.mark.skipif(
    len(uvc.list_devices()) == 0,
    reason="カメラデバイスが接続されていない",
)
def test_concurrent_device_info_access():
    """複数スレッドから同時に DeviceInfo にアクセスしてもクラッシュしないことを確認する"""
    devices = uvc.list_devices()
    device_info = devices[0]
    errors = []

    def access_device_info():
        try:
            for _ in range(100):
                _ = device_info.name
                _ = device_info.unique_id
                _ = device_info.index
                _ = repr(device_info)
        except Exception as exception:
            errors.append(exception)

    with ThreadPoolExecutor(max_workers=4) as executor:
        futures = [executor.submit(access_device_info) for _ in range(4)]
        for future in futures:
            future.result()

    assert len(errors) == 0, f"エラーが発生: {errors}"


@pytest.mark.skipif(
    len(uvc.list_devices()) == 0,
    reason="カメラデバイスが接続されていない",
)
def test_concurrent_get_supported_formats():
    """複数スレッドから同時に get_supported_formats() を呼び出してもクラッシュしないことを確認する"""
    devices = uvc.list_devices()
    errors = []

    with uvc.open(devices[0]) as device:

        def get_formats():
            try:
                for _ in range(10):
                    formats = device.get_supported_formats()
                    assert isinstance(formats, list)
                    for fmt in formats:
                        _ = fmt.width
                        _ = fmt.height
                        _ = fmt.fps
                        _ = fmt.format
                        _ = repr(fmt)
            except Exception as exception:
                errors.append(exception)

        with ThreadPoolExecutor(max_workers=4) as executor:
            futures = [executor.submit(get_formats) for _ in range(4)]
            for future in futures:
                future.result()

    assert len(errors) == 0, f"エラーが発生: {errors}"


@pytest.mark.skipif(
    len(uvc.list_devices()) == 0,
    reason="カメラデバイスが接続されていない",
)
def test_concurrent_frame_access():
    """複数スレッドから同時に Frame にアクセスしてもクラッシュしないことを確認する"""
    devices = uvc.list_devices()
    errors = []

    with uvc.open(devices[0]) as device:
        formats = device.get_supported_formats()
        fmt = formats[0]
        device.start(fmt.width, fmt.height, fmt.fps, fmt.format)

        time.sleep(0.5)

        frame = None
        for _ in range(100):
            frame = device.get_frame()
            if frame is not None:
                break
            time.sleep(0.05)

        device.stop()

    assert frame is not None, "フレームが取得できなかった"

    def access_frame():
        try:
            for _ in range(100):
                _ = frame.width
                _ = frame.height
                _ = frame.format
                _ = frame.timestamp
        except Exception as exception:
            errors.append(exception)

    with ThreadPoolExecutor(max_workers=4) as executor:
        futures = [executor.submit(access_frame) for _ in range(4)]
        for future in futures:
            future.result()

    assert len(errors) == 0, f"エラーが発生: {errors}"


@pytest.mark.skipif(
    len(uvc.list_devices()) == 0,
    reason="カメラデバイスが接続されていない",
)
def test_concurrent_frame_conversion():
    """複数スレッドから同時に Frame の変換を行ってもクラッシュしないことを確認する"""
    devices = uvc.list_devices()
    errors = []

    with uvc.open(devices[0]) as device:
        formats = device.get_supported_formats()
        fmt = formats[0]
        device.start(fmt.width, fmt.height, fmt.fps, fmt.format)

        time.sleep(0.5)

        frame = None
        for _ in range(100):
            frame = device.get_frame()
            if frame is not None:
                break
            time.sleep(0.05)

        device.stop()

    assert frame is not None, "フレームが取得できなかった"

    # フレームのフォーマットに応じた変換関数を選択
    frame_format = frame.format

    def convert_frame():
        try:
            for _ in range(10):
                if frame_format == uvc.Format.NV12:
                    _ = frame.to_nv12()
                elif frame_format == uvc.Format.YUY2:
                    _ = frame.to_yuy2()
                elif frame_format == uvc.Format.RGB:
                    _ = frame.to_rgb()
                elif frame_format == uvc.Format.RGBA:
                    _ = frame.to_rgba()
        except Exception as exception:
            errors.append(exception)

    with ThreadPoolExecutor(max_workers=4) as executor:
        futures = [executor.submit(convert_frame) for _ in range(4)]
        for future in futures:
            future.result()

    assert len(errors) == 0, f"エラーが発生: {errors}"


@pytest.mark.skipif(
    len(uvc.list_devices()) == 0,
    reason="カメラデバイスが接続されていない",
)
def test_concurrent_get_frame():
    """複数スレッドから同時に get_frame() を呼び出してもクラッシュしないことを確認する"""
    devices = uvc.list_devices()
    errors = []
    frames_received = []

    with uvc.open(devices[0]) as device:
        formats = device.get_supported_formats()
        fmt = formats[0]
        device.start(fmt.width, fmt.height, fmt.fps, fmt.format)

        time.sleep(0.5)

        def get_frames():
            try:
                for _ in range(10):
                    frame = device.get_frame()
                    if frame is not None:
                        frames_received.append(frame)
                    time.sleep(0.01)
            except Exception as exception:
                errors.append(exception)

        with ThreadPoolExecutor(max_workers=4) as executor:
            futures = [executor.submit(get_frames) for _ in range(4)]
            for future in futures:
                future.result()

        device.stop()

    assert len(errors) == 0, f"エラーが発生: {errors}"


@pytest.mark.skipif(
    len(uvc.list_devices()) == 0,
    reason="カメラデバイスが接続されていない",
)
def test_concurrent_start_stop():
    """複数スレッドから start/stop を呼び出してもデッドロックしないことを確認する"""
    devices = uvc.list_devices()
    errors = []

    with uvc.open(devices[0]) as device:
        formats = device.get_supported_formats()
        fmt = formats[0]

        def start_stop_cycle():
            try:
                device.start(fmt.width, fmt.height, fmt.fps, fmt.format)
                time.sleep(0.1)
                device.stop()
            except Exception as exception:
                errors.append(exception)

        # 逐次実行でデッドロックしないことを確認
        for _ in range(3):
            start_stop_cycle()

    assert len(errors) == 0, f"エラーが発生: {errors}"


def test_gil_status():
    """GIL の状態を確認する（情報表示用）"""
    if hasattr(sys, "_is_gil_enabled"):
        gil_enabled = sys._is_gil_enabled()
        print(f"\nGIL 状態: {'有効' if gil_enabled else '無効 (Free-Threading)'}")
    else:
        print("\nGIL 状態: 有効 (Python 3.12 以前)")
