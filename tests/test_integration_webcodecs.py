"""uvc-py と webcodecs-py の統合テスト."""

import platform
import time

import numpy as np
import pytest

import uvc

# webcodecs-py が利用可能かチェック
pytest.importorskip("webcodecs")

from webcodecs import (
    HardwareAccelerationEngine,
    LatencyMode,
    VideoEncoder,
    VideoEncoderConfig,
    VideoFrame,
    VideoPixelFormat,
)


@pytest.mark.skip(reason="webcodecs-py 側の native_buffer 対応待ち")
@pytest.mark.skipif(
    len(uvc.list_devices()) == 0,
    reason="カメラデバイスが接続されていない",
)
@pytest.mark.skipif(
    platform.system() != "Darwin",
    reason="macOS でのみ実行する",
)
def test_webcodecs_native_buffer_zero_copy_encode():
    """native_buffer を使ったゼロコピーエンコードのテスト."""
    devices = uvc.list_devices()

    with uvc.open(devices[0]) as device:
        # NV12 フォーマットを探す
        formats = device.get_supported_formats()
        nv12_formats = [f for f in formats if f.format == uvc.Format.NV12]

        if not nv12_formats:
            pytest.skip("NV12 フォーマットが利用できない")

        fmt = nv12_formats[0]
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
    assert frame.format == uvc.Format.NV12

    # フレームの実際のサイズを使用
    width = frame.width
    height = frame.height

    # native_buffer を取得
    native = frame.native_buffer()
    assert native is not None

    # エンコーダーを作成
    encoded_chunks = []

    def on_output(chunk):
        encoded_chunks.append(chunk)

    def on_error(error):
        print(f"Encoder error: {error}")

    encoder = VideoEncoder(on_output, on_error)

    config: VideoEncoderConfig = {
        "codec": "avc1.640028",
        "width": width,
        "height": height,
        "bitrate": 5_000_000,
        "framerate": 30,
        "latency_mode": LatencyMode.REALTIME,
        "hardware_acceleration_engine": HardwareAccelerationEngine.APPLE_VIDEO_TOOLBOX,
        "avc": {"format": "annexb"},
    }
    encoder.configure(config)

    # uvc-py のフレームから webcodecs-py の VideoFrame を作成
    # ゼロコピーの場合、data は使用されないがコンストラクタには必要
    y, uv = frame.to_nv12()
    # ストライドを考慮して contiguous なデータを作成
    y_data = np.ascontiguousarray(y)
    uv_data = np.ascontiguousarray(uv)
    data = np.concatenate([y_data.flatten(), uv_data.flatten()])

    video_frame = VideoFrame(
        data,
        {
            "format": VideoPixelFormat.NV12,
            "coded_width": width,
            "coded_height": height,
            "timestamp": frame.timestamp,
        },
    )

    # native_buffer を設定（ゼロコピー）
    video_frame.native_buffer = native

    # エンコード
    encoder.encode(video_frame, {"key_frame": True})
    encoder.flush()
    encoder.close()

    # エンコードされたチャンクがあることを確認
    assert len(encoded_chunks) > 0
    print(f"Encoded {len(encoded_chunks)} chunks (zero-copy)")
    for chunk in encoded_chunks:
        print(f"  - {chunk.type}, size={chunk.byte_length}")


@pytest.mark.skip(reason="webcodecs-py 側の native_buffer 対応待ち")
@pytest.mark.skipif(
    len(uvc.list_devices()) == 0,
    reason="カメラデバイスが接続されていない",
)
@pytest.mark.skipif(
    platform.system() != "Darwin",
    reason="macOS でのみ実行する",
)
def test_webcodecs_native_buffer_fallback():
    """native_buffer なしでも動作することを確認するテスト（フォールバック）."""
    devices = uvc.list_devices()

    with uvc.open(devices[0]) as device:
        # NV12 フォーマットを探す
        formats = device.get_supported_formats()
        nv12_formats = [f for f in formats if f.format == uvc.Format.NV12]

        if not nv12_formats:
            pytest.skip("NV12 フォーマットが利用できない")

        fmt = nv12_formats[0]
        device.start(fmt.width, fmt.height, fmt.fps, fmt.format)

        time.sleep(0.5)

        frame = None
        for _ in range(100):
            frame = device.get_frame()
            if frame is not None:
                break
            time.sleep(0.05)

    assert frame is not None

    # フレームの実際のサイズを使用
    width = frame.width
    height = frame.height

    # エンコーダーを作成
    encoded_chunks = []

    def on_output(chunk):
        encoded_chunks.append(chunk)

    def on_error(error):
        print(f"Encoder error: {error}")

    encoder = VideoEncoder(on_output, on_error)

    config: VideoEncoderConfig = {
        "codec": "avc1.640028",
        "width": width,
        "height": height,
        "bitrate": 5_000_000,
        "framerate": 30,
        "latency_mode": LatencyMode.REALTIME,
        "hardware_acceleration_engine": HardwareAccelerationEngine.APPLE_VIDEO_TOOLBOX,
        "avc": {"format": "annexb"},
    }
    encoder.configure(config)

    # native_buffer を設定せずに VideoFrame を作成（フォールバックパス）
    y, uv = frame.to_nv12()
    # ストライドを考慮して contiguous なデータを作成
    y_data = np.ascontiguousarray(y)
    uv_data = np.ascontiguousarray(uv)
    data = np.concatenate([y_data.flatten(), uv_data.flatten()])

    video_frame = VideoFrame(
        data,
        {
            "format": VideoPixelFormat.NV12,
            "coded_width": width,
            "coded_height": height,
            "timestamp": frame.timestamp,
        },
    )

    # native_buffer を設定しない（フォールバック）
    assert video_frame.native_buffer is None

    # エンコード
    encoder.encode(video_frame, {"key_frame": True})
    encoder.flush()
    encoder.close()

    # エンコードされたチャンクがあることを確認
    assert len(encoded_chunks) > 0
    print(f"Encoded {len(encoded_chunks)} chunks (fallback path)")
