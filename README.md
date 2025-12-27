# uvc-py

[![PyPI](https://img.shields.io/pypi/v/uvc-py)](https://pypi.org/project/uvc-py/)
[![SPEC 0 — Minimum Supported Dependencies](https://img.shields.io/badge/SPEC-0-green?labelColor=%23004811&color=%235CA038)](https://scientific-python.org/specs/spec-0000/)
[![image](https://img.shields.io/pypi/pyversions/uvc-py.svg)](https://pypi.python.org/pypi/uvc-py)
[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

## About Shiguredo's open source software

We will not respond to PRs or issues that have not been discussed on Discord. Also, Discord is only available in Japanese.

Please read <https://github.com/shiguredo/oss/blob/master/README.en.md> before use.

## 時雨堂のオープンソースソフトウェアについて

利用前に <https://github.com/shiguredo/oss> をお読みください。

## uvc-py について

UVC (USB Video Class) カメラからの映像キャプチャを行う Python ライブラリです。

- numpy.ndarray での映像フレーム取得
- ネイティブバッファへの直接アクセス (macOS: CVPixelBufferRef / PyCapsule)
- デバイス接続/切断コールバック (macOS / Linux)
- Python [Free Threading](https://docs.python.org/3/howto/free-threading-python.html) 対応

## 対応フォーマット

- NV12
- YUY2

### 将来サポート予定

- MJPEG

## 対応プラットフォーム

- macOS 26 arm64
- macOS 15 arm64
- Ubuntu 24.04 x86_64
- Ubuntu 24.04 arm64
- Ubuntu 22.04 x86_64
- Ubuntu 22.04 arm

| OS | API |
| --- | --- |
| macOS | AVFoundation |
| Linux | V4L2 |
| Windows | Media Foundation |

### 将来対応予定

- Windows Server 2025 x86_64
- Windows 11 x86_64

## 対応 Python

- 3.14
- 3.14t
- 3.13
- 3.13t
- 3.12

## インストール

```bash
uv add uvc-py
```

## 使い方

### デバイス一覧の取得

```python
import uvc

devices = uvc.list_devices()
for device in devices:
    print(f"[{device.index}] {device.name}")
```

### 映像キャプチャ

```python
import uvc

with uvc.open(0) as device:
    device.start(1920, 1080, 30, capture_format=uvc.Format.NV12)

    frame = device.get_frame()
    if frame is not None:
        y, uv = frame.to_nv12()
        print(f"Y: {y.shape}")
        print(f"UV: {uv.shape}")
```

### デバイス接続/切断コールバック

```python
import uvc
import time

def on_connected():
    print("デバイスが接続されました")

def on_disconnected():
    print("デバイスが切断されました")

with uvc.open(0, on_connected=on_connected, on_disconnected=on_disconnected) as device:
    device.start(1920, 1080, 30, capture_format=uvc.Format.NV12)

    while True:
        frame = device.get_frame()
        if frame is not None:
            print(f"Frame: {frame.width}x{frame.height}")
        time.sleep(0.033)
```

### raw-player との連携

```python
import uvc
from raw_player import VideoPlayer

with uvc.open(0) as device:
    device.start(1920, 1080, 30, capture_format=uvc.Format.NV12)

    player = VideoPlayer(width=1920, height=1080)
    player.play()

    while player.is_open:
        if not player.poll_events():
            break

        frame = device.get_frame()
        if frame is not None:
            y, uv = frame.to_nv12()
            player.enqueue_video_nv12(y, uv, frame.timestamp)

    player.close()
```

### webcodecs-py との連携

```python
import uvc
from webcodecs import VideoEncoder, VideoFrame, VideoPixelFormat

def on_output(chunk):
    print(f"Encoded: {chunk.byte_length} bytes")

def on_error(error):
    print(f"Error: {error}")

with uvc.open(0) as device:
    device.start(1920, 1080, 30, capture_format=uvc.Format.NV12)

    encoder = VideoEncoder(on_output, on_error)
    encoder.configure({
        "codec": "avc1.640028",
        "width": 1920,
        "height": 1080,
        "bitrate": 5_000_000,
        "framerate": 30,
    })

    frame = device.get_frame()
    if frame is not None:
        y, uv = frame.to_nv12()
        with VideoFrame(
            y,
            uv,
            {
                "format": VideoPixelFormat.NV12,
                "coded_width": frame.width,
                "coded_height": frame.height,
                "timestamp": frame.timestamp,
            },
        ) as video_frame:
            encoder.encode(video_frame, {"key_frame": True})

    encoder.flush()
    encoder.close()
```

## PyCapsule

[PyCapsule](https://docs.python.org/3/c-api/capsule.html) は Python C API の仕組みで、C/C++ のポインタを Python オブジェクトとして安全に受け渡しできます。

uvc-py では `frame.native_buffer()` で macOS の CVPixelBufferRef を PyCapsule として取得できます。
これにより他の C 拡張モジュールが CVPixelBufferRef を直接扱えます。

> [!NOTE]
>
> - Linux / Windows では `native_buffer()` は None を返します

### raw-player との連携

`native_buffer()` を渡すことで CVPixelBufferRef を直接扱えます。

```python
import uvc
from raw_player import VideoPlayer

with uvc.open(0) as device:
    device.start(1920, 1080, 30, capture_format=uvc.Format.NV12)

    player = VideoPlayer(width=1920, height=1080)
    player.play()

    while player.is_open:
        if not player.poll_events():
            break

        frame = device.get_frame()
        if frame is not None:
            player.enqueue_video_nv12(frame.native_buffer(), frame.timestamp)

    player.close()
```

### webcodecs-py との連携

`native_buffer()` を渡すことで CVPixelBufferRef を直接扱えます。

```python
import uvc
from webcodecs import VideoEncoder, VideoFrame, VideoPixelFormat

def on_output(chunk):
    print(f"Encoded: {chunk.byte_length} bytes")

def on_error(error):
    print(f"Error: {error}")

with uvc.open(0) as device:
    device.start(1920, 1080, 30, capture_format=uvc.Format.NV12)

    encoder = VideoEncoder(on_output, on_error)
    encoder.configure({
        "codec": "avc1.640028",
        "width": 1920,
        "height": 1080,
        "bitrate": 5_000_000,
        "framerate": 30,
    })

    frame = device.get_frame()
    if frame is not None:
        with VideoFrame(
            frame.native_buffer(),
            {
                "format": VideoPixelFormat.NV12,
                "coded_width": frame.width,
                "coded_height": frame.height,
                "timestamp": frame.timestamp,
            },
        ) as video_frame:
            encoder.encode(video_frame, {"key_frame": True})

    encoder.flush()
    encoder.close()
```

## API リファレンス

### モジュール関数

| 関数 | 説明 |
|------|------|
| `list_devices()` | 利用可能な UVC デバイスの一覧を取得 |
| `open(index_or_info, on_connected, on_disconnected)` | デバイスを開く |

#### open の引数

- `index_or_info`: デバイスインデックス(int)または DeviceInfo
- `on_connected`: デバイス接続時のコールバック (macOS / Linux)
- `on_disconnected`: デバイス切断時のコールバック (macOS / Linux)

### Device

UVC デバイスを操作するクラス。コンテキストマネージャとして使用可能。

```python
with uvc.open(0) as device:
    device.start(1920, 1080, 30, capture_format=uvc.Format.NV12)
```

| メソッド | 説明 |
|----------|------|
| `start(width, height, fps, capture_format, output_format)` | キャプチャ開始 |
| `stop()` | キャプチャ停止 |
| `get_frame()` | フレーム取得(None の場合あり) |
| `get_supported_formats()` | サポートされているフォーマット一覧を取得 |

| プロパティ | 説明 |
|------------|------|
| `is_running` | キャプチャ中かどうか |
| `info` | デバイス情報(DeviceInfo) |

#### start の引数

- `width`: 映像幅
- `height`: 映像高さ
- `fps`: フレームレート
- `capture_format`: キャプチャフォーマット(Format)
- `output_format`: 出力フォーマット(省略時は capture_format と同じ)

### Frame

キャプチャしたフレームを表すクラス。

| メソッド | 説明 |
|----------|------|
| `to_nv12()` | NV12 フォーマット: `(y, uv)` タプルを返す |
| `to_yuy2()` | YUY2 フォーマット: `(H, W, 2)` を返す |
| `to_rgb()` | RGB フォーマット: `(H, W, 3)` を返す |
| `to_rgba()` | RGBA フォーマット: `(H, W, 4)` を返す |
| `native_buffer()` | ネイティブバッファを PyCapsule として取得 |

| プロパティ | 説明 |
|------------|------|
| `width` | フレーム幅 |
| `height` | フレーム高さ |
| `format` | フォーマット(Format) |
| `timestamp` | タイムスタンプ(マイクロ秒) |

#### to_nv12 の戻り値

- `y`: Y プレーン(uint8、shape: `(H, W)`)
- `uv`: UV インターリーブプレーン(uint8、shape: `(H/2, W)`)

### DeviceInfo

デバイス情報を表すクラス。

| プロパティ | 説明 |
|------------|------|
| `name` | デバイス名 |
| `unique_id` | ユニーク ID |
| `index` | デバイスインデックス |

### FormatInfo

フォーマット情報を表すクラス。

| プロパティ | 説明 |
|------------|------|
| `width` | 映像幅 |
| `height` | 映像高さ |
| `fps` | フレームレート |
| `format` | フォーマット(Format) |

### Format

映像フォーマットを表す列挙型。

| 値 | 説明 |
|-----|------|
| `NV12` | NV12(Y + UV インターリーブ) |
| `YUY2` | YUY2(パックド YUV) |
| `RGB` | RGB |
| `RGBA` | RGBA |
| `MJPEG` | Motion JPEG(将来サポート予定) |

## ビルド

```bash
make develop
```

## サンプルの実行

```bash
uv sync --only-group example
make develop
```

```bash
# デバイス一覧を表示
uv run examples/capture.py --list-devices

# 720p 30fps でキャプチャ
uv run examples/capture.py --resolution 720p --fps 30

# NV12 フォーマットでキャプチャ
uv run examples/capture.py --capture-format nv12

# raw-player で映像を表示
uv run examples/capture.py --player

# 100 フレームだけキャプチャ
uv run examples/capture.py --frames 100

# 10 秒間キャプチャ
uv run examples/capture.py --duration 10
```

## Discord

- **サポートはしません**
- アドバイスします
- フィードバック歓迎します

最新の状況などは Discord で共有しています。質問や相談も Discord でのみ受け付けています。

<https://discord.gg/shiguredo>

## バグ報告

Discord へお願いします。

## ライセンス

Apache License 2.0

```text
Copyright 2025-2025, Shiguredo Inc.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```
