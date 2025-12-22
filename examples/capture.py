#!/usr/bin/env python3
"""UVC カメラキャプチャのサンプル"""

import argparse
import sys
import time

import uvc

IS_MACOS = sys.platform == "darwin"


def parse_args():
    parser = argparse.ArgumentParser(description="UVC カメラキャプチャのサンプル")
    parser.add_argument(
        "--video-device",
        type=str,
        default=None,
        help="デバイスインデックス、unique_id、またはデバイス名",
    )
    parser.add_argument(
        "--capture-format",
        type=str,
        default=None,
        choices=["yuy2", "nv12", "rgb", "rgba"],
        help="キャプチャフォーマット (デフォルト: 自動選択)",
    )
    parser.add_argument(
        "--output-format",
        type=str,
        default=None,
        choices=["rgb", "rgba", "nv12"],
        help="出力フォーマット (デフォルト: capture-format と同じ)",
    )
    parser.add_argument(
        "--resolution",
        type=str,
        default="720p",
        help="解像度 (例: 1080p, 720p, 480p, 4k, 1920x1080)",
    )
    parser.add_argument(
        "--fps",
        type=int,
        default=30,
        help="フレームレート (デフォルト: 30)",
    )
    parser.add_argument(
        "--frames",
        type=int,
        default=None,
        help="キャプチャするフレーム数",
    )
    parser.add_argument(
        "--duration",
        type=float,
        default=None,
        help="キャプチャ時間 (秒)",
    )
    parser.add_argument(
        "--list-devices",
        action="store_true",
        help="デバイス一覧と対応フォーマットを表示して終了",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="--list-devices と併用して JSON 形式で出力",
    )
    parser.add_argument(
        "--player",
        action="store_true",
        help="raw-player で映像を表示",
    )
    if IS_MACOS:
        parser.add_argument(
            "--native-buffer",
            action="store_true",
            default=False,
            help="CVPixelBuffer を直接使用 (デフォルト: 無効, macOS のみ)",
        )
    return parser.parse_args()


def parse_resolution(resolution_str: str) -> tuple[int, int]:
    """解像度文字列をパースして (width, height) を返す"""
    resolution_str = resolution_str.lower()

    # プリセット解像度
    presets = {
        "4k": (3840, 2160),
        "2160p": (3840, 2160),
        "1440p": (2560, 1440),
        "1080p": (1920, 1080),
        "720p": (1280, 720),
        "540p": (960, 540),
        "480p": (640, 480),
        "360p": (640, 360),
        "240p": (320, 240),
    }

    if resolution_str in presets:
        return presets[resolution_str]

    # WxH 形式
    if "x" in resolution_str:
        parts = resolution_str.split("x")
        if len(parts) == 2:
            return (int(parts[0]), int(parts[1]))

    raise ValueError(f"Invalid resolution format: {resolution_str}")


def get_format_enum(format_str: str | None) -> uvc.Format | None:
    match format_str:
        case None:
            return None
        case s if s.lower() == "yuy2":
            return uvc.Format.YUY2
        case s if s.lower() == "nv12":
            return uvc.Format.NV12
        case s if s.lower() == "rgb":
            return uvc.Format.RGB
        case s if s.lower() == "rgba":
            return uvc.Format.RGBA
        case _:
            return None


def format_to_str(fmt: uvc.Format) -> str:
    match fmt:
        case uvc.Format.YUY2:
            return "YUY2"
        case uvc.Format.NV12:
            return "NV12"
        case uvc.Format.RGB:
            return "RGB"
        case uvc.Format.RGBA:
            return "RGBA"
        case _:
            return "Unknown"


def list_devices_detail(output_json: bool = False):
    """デバイス一覧と対応フォーマットを詳細表示"""
    import json

    devices = uvc.list_devices()
    if not devices:
        if output_json:
            print(json.dumps([], indent=2))
        else:
            print("No devices found")
        return

    if output_json:
        result = []
        for dev_info in devices:
            dev = uvc.open(dev_info.index)
            formats = dev.get_supported_formats()

            # フォーマット毎にグループ化
            format_groups: dict[uvc.Format, list[tuple[int, int, int]]] = {}
            for fmt in formats:
                if fmt.format not in format_groups:
                    format_groups[fmt.format] = []
                resolution = (fmt.width, fmt.height, fmt.fps)
                if resolution not in format_groups[fmt.format]:
                    format_groups[fmt.format].append(resolution)

            formats_json = {}
            for fmt_type, resolutions in format_groups.items():
                fmt_name = format_to_str(fmt_type)
                formats_json[fmt_name] = [
                    {"width": w, "height": h, "fps": fps} for w, h, fps in resolutions
                ]

            result.append(
                {
                    "index": dev_info.index,
                    "name": dev_info.name,
                    "unique_id": dev_info.unique_id,
                    "formats": formats_json,
                }
            )

        print(json.dumps(result, indent=2, ensure_ascii=False))
        return

    for dev_info in devices:
        print(f"[{dev_info.index}] {dev_info.name}")
        print(f"    ID: {dev_info.unique_id}")

        dev = uvc.open(dev_info.index)
        formats = dev.get_supported_formats()

        # フォーマット毎にグループ化
        format_groups: dict[uvc.Format, list[tuple[int, int, int]]] = {}
        for fmt in formats:
            if fmt.format not in format_groups:
                format_groups[fmt.format] = []
            resolution = (fmt.width, fmt.height, fmt.fps)
            if resolution not in format_groups[fmt.format]:
                format_groups[fmt.format].append(resolution)

        def resolution_pixels(r):
            return r[0] * r[1]

        def resolution_item_pixels(item):
            return item[0][0] * item[0][1]

        # 各フォーマットの情報を表示
        print("    Formats:")
        for fmt_type, resolutions in format_groups.items():
            # 最大解像度を取得
            max_resolution = max(resolutions, key=resolution_pixels)
            # 最大 fps を取得
            max_fps = max(r[2] for r in resolutions)
            # 最大 fps が出る解像度を取得
            max_fps_resolution = max(
                (r for r in resolutions if r[2] == max_fps), key=resolution_pixels
            )

            print(f"      {format_to_str(fmt_type)}:")
            print(
                f"        Max resolution: {max_resolution[0]}x{max_resolution[1]}@{max_resolution[2]}fps"
            )
            if max_fps != max_resolution[2]:
                print(
                    f"        Max fps: {max_fps_resolution[0]}x{max_fps_resolution[1]}@{max_fps}fps"
                )

            # 解像度毎の最大 fps を表示
            resolution_fps: dict[tuple[int, int], int] = {}
            for w, h, fps in resolutions:
                key = (w, h)
                if key not in resolution_fps or fps > resolution_fps[key]:
                    resolution_fps[key] = fps

            sorted_resolutions = sorted(
                resolution_fps.items(), key=resolution_item_pixels, reverse=True
            )
            res_strs = [f"{w}x{h}@{fps}" for (w, h), fps in sorted_resolutions]
            print(f"        All: {', '.join(res_strs)}")

        print()


def run_with_player(dev, selected, output_format, max_frames, duration, use_native_buffer):
    """raw-player で映像を表示"""
    from raw_player import VideoPlayer, get_gpu_driver, get_version

    print(f"SDL Version: {get_version()}")
    print(f"GPU Driver: {get_gpu_driver()}")

    player = VideoPlayer(
        width=selected.width,
        height=selected.height,
        title=f"UVC Camera ({selected.width}x{selected.height}@{selected.fps}fps)",
    )

    def on_key(key: int) -> bool:
        # ESC (27) または q (113) で終了
        if key == 27 or key == 113:
            return False
        return True

    player.set_key_callback(on_key)
    player.play()

    print(f"GPU Renderer: {player.renderer_name}")
    print("ESC または q キーで終了")
    print()

    frame_count = 0
    start_time = time.time()

    def should_continue():
        if not player.is_open:
            return False
        if max_frames is not None and frame_count >= max_frames:
            return False
        if duration is not None and (time.time() - start_time) >= duration:
            return False
        return True

    try:
        while should_continue():
            if not player.poll_events():
                break

            frame = dev.get_frame()
            if frame is not None:
                frame_count += 1
                timestamp_us = frame.timestamp

                if frame_count == 1:
                    print(f"Frame format: {frame.format}, expected: {output_format}")

                match frame.format:
                    case uvc.Format.NV12:
                        native_buf = frame.native_buffer() if use_native_buffer else None
                        if native_buf is not None:
                            if frame_count == 1:
                                print(f"Native buffer: {native_buf}")
                            player.enqueue_video_nv12(native_buf, timestamp_us)
                        else:
                            y, uv = frame.to_nv12()
                            if frame_count == 1:
                                print(
                                    "Native buffer: disabled"
                                    if not use_native_buffer
                                    else "Native buffer: None"
                                )
                                print(f"Y: {y.shape}, UV: {uv.shape}")
                            player.enqueue_video_nv12(y, uv, timestamp_us)
                    case uvc.Format.YUY2:
                        native_buf = frame.native_buffer() if use_native_buffer else None
                        if native_buf is not None:
                            if frame_count == 1:
                                print(f"Native buffer: {native_buf}")
                            player.enqueue_video_yuy2(native_buf, timestamp_us)
                        else:
                            yuy2 = frame.to_yuy2()
                            if frame_count == 1:
                                print(
                                    "Native buffer: disabled"
                                    if not use_native_buffer
                                    else "Native buffer: None"
                                )
                                print(f"YUY2: {yuy2.shape}")
                            player.enqueue_video_yuy2(yuy2, timestamp_us)
                    case _:
                        print(f"Warning: {frame.format} is not supported with --player")
                        break

                if frame_count % 30 == 0:
                    elapsed = time.time() - start_time
                    actual_fps = frame_count / elapsed
                    stats = player.stats()
                    print(
                        f"Frame {frame_count}: {actual_fps:.1f} fps, "
                        f"queue={stats['video_queue_size']}, "
                        f"dropped={stats['dropped_frames']}"
                    )
            else:
                time.sleep(0.001)

    except KeyboardInterrupt:
        print("\nInterrupted")

    # 統計
    elapsed = time.time() - start_time
    print("\n=== Stats ===")
    print(f"Frames: {frame_count}")
    print(f"Time: {elapsed:.2f}s")
    if elapsed > 0:
        print(f"FPS: {frame_count / elapsed:.1f}")

    print("\nPlayer stats:")
    stats = player.stats()
    for key, value in stats.items():
        print(f"  {key}: {value}")

    player.close()


def run_without_player(dev, selected, output_format, max_frames, duration):
    """プレイヤーなしでキャプチャのみ"""
    frame_count = 0
    start_time = time.time()

    def should_continue():
        if max_frames is not None and frame_count >= max_frames:
            return False
        if duration is not None and (time.time() - start_time) >= duration:
            return False
        return True

    try:
        while should_continue():
            frame = dev.get_frame()
            if frame is not None:
                frame_count += 1

                match frame.format:
                    case uvc.Format.NV12:
                        y, uv = frame.to_nv12()
                        if frame_count == 1:
                            print("Frame format: NV12")
                            print(f"Y: {y.shape}, UV: {uv.shape}")
                    case uvc.Format.YUY2:
                        yuy2 = frame.to_yuy2()
                        if frame_count == 1:
                            print("Frame format: YUY2")
                            print(f"YUY2: {yuy2.shape}")
                    case _:
                        pass

                if frame_count % 10 == 0:
                    elapsed = time.time() - start_time
                    actual_fps = frame_count / elapsed
                    print(f"Frame {frame_count}: {actual_fps:.1f} fps")
            else:
                time.sleep(0.001)

    except KeyboardInterrupt:
        print("\nInterrupted")

    # 統計
    elapsed = time.time() - start_time
    print(f"\n=== Stats ===")
    print(f"Frames: {frame_count}")
    print(f"Time: {elapsed:.2f}s")
    if elapsed > 0:
        print(f"FPS: {frame_count / elapsed:.1f}")


def main():
    args = parse_args()

    if args.list_devices:
        list_devices_detail(args.json)
        return

    # デバイス一覧を表示
    print("=== Available Devices ===")
    devices = uvc.list_devices()
    for dev in devices:
        print(f"  [{dev.index}] {dev.name}")

    if not devices:
        print("No devices found")
        return

    # デバイスを選択
    target_format = get_format_enum(args.capture_format)
    selected_device_info: uvc.DeviceInfo | None = None

    if args.video_device is not None:
        # 数値かどうかを判定
        try:
            device_index = int(args.video_device, 0)
            # 整数として解釈できた場合、範囲内なら index として扱う
            if 0 <= device_index < len(devices):
                selected_device_info = devices[device_index]
        except ValueError:
            pass

        # index で見つからなければ unique_id として探す
        if selected_device_info is None:
            for dev_info in devices:
                if dev_info.unique_id == args.video_device:
                    selected_device_info = dev_info
                    break

        # unique_id で見つからなければデバイス名として探す
        if selected_device_info is None:
            for dev_info in devices:
                if dev_info.name == args.video_device:
                    selected_device_info = dev_info
                    break

        if selected_device_info is None:
            print(f"Device '{args.video_device}' not found")
            return

    # デバイスが指定されていないがフォーマットが指定されている場合、
    # そのフォーマットをサポートするデバイスを自動選択
    if selected_device_info is None and target_format is not None:
        for dev_info in devices:
            temp_dev = uvc.open(dev_info)
            formats = temp_dev.get_supported_formats()
            if any(fmt.format == target_format for fmt in formats):
                selected_device_info = dev_info
                print(
                    f"\nAuto-selected device {dev_info.index} ({dev_info.name}) for {args.capture_format.upper()}"
                )
                break

    # それでも見つからなければ最初のデバイスを使用
    if selected_device_info is None:
        selected_device_info = devices[0]

    print(
        f"\n=== Opening device {selected_device_info.index} ({selected_device_info.unique_id}) ==="
    )
    dev = uvc.open(selected_device_info)
    print(f"Device: {dev.info.name}")

    # サポートされているフォーマットを表示
    print("\n=== Supported Formats ===")
    formats = dev.get_supported_formats()
    seen = set()
    for fmt in formats:
        key = (fmt.width, fmt.height, fmt.fps, fmt.format)
        if key not in seen:
            seen.add(key)
            print(f"  {fmt}")

    # キャプチャ開始（利用可能なフォーマットから選択）
    target_width, target_height = parse_resolution(args.resolution)
    target_fps = args.fps
    target_format = get_format_enum(args.capture_format)
    output_format = get_format_enum(args.output_format)

    selected = None

    # 指定されたフォーマットで探す
    for fmt in formats:
        width_match = fmt.width == target_width
        height_match = fmt.height == target_height
        fps_match = fmt.fps == target_fps
        format_match = target_format is None or fmt.format == target_format

        if width_match and height_match and fps_match and format_match:
            selected = fmt
            break

    if not selected and formats:
        # 見つからなければフォーマット指定のみで探す
        if target_format is not None:
            for fmt in formats:
                if fmt.format == target_format:
                    selected = fmt
                    break

        # それでも見つからなければ最初の 30fps フォーマットを使用
        if not selected:
            for fmt in formats:
                if fmt.fps == 30:
                    selected = fmt
                    break

        if not selected:
            selected = formats[0]

    if not selected:
        print("No formats available")
        return

    # output_format が指定されていない場合は capture format を使用
    if output_format is None:
        output_format = selected.format

    print(f"\n=== Starting capture: {selected} -> {format_to_str(output_format)} ===")
    dev.start(selected.width, selected.height, selected.fps, selected.format, output_format)

    # フレーム取得
    print("\n=== Capturing frames ===")
    max_frames = args.frames
    duration = args.duration

    # どちらも指定されていない場合
    if max_frames is None and duration is None:
        if args.player:
            # プレイヤー使用時は無制限
            pass
        else:
            # プレイヤーなしの場合はデフォルトで 100 フレーム
            max_frames = 100

    # native_buffer オプションは macOS のみ
    use_native_buffer = getattr(args, "native_buffer", False)

    if args.player:
        run_with_player(dev, selected, output_format, max_frames, duration, use_native_buffer)
    else:
        run_without_player(dev, selected, output_format, max_frames, duration)

    # 停止
    dev.stop()
    print("\nDone")


if __name__ == "__main__":
    main()
