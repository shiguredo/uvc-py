cmake_minimum_required(VERSION 4.2)

if(APPLE)
    project(uvc_py LANGUAGES CXX C OBJCXX)
else()
    project(uvc_py LANGUAGES CXX C)
endif()

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

# プラットフォームの検出
if(APPLE)
    if(NOT CMAKE_OSX_DEPLOYMENT_TARGET)
        set(CMAKE_OSX_DEPLOYMENT_TARGET "16.0" CACHE STRING "Minimum macOS deployment version")
    endif()
    message(STATUS "macOS deployment target: ${CMAKE_OSX_DEPLOYMENT_TARGET}")
elseif(WIN32)
    message(STATUS "Building for Windows")
elseif(UNIX)
    message(STATUS "Building for Linux")
else()
    message(FATAL_ERROR "Unsupported platform.")
endif()

# Python と nanobind を検索
find_package(Python REQUIRED COMPONENTS Interpreter Development.Module)
find_package(nanobind CONFIG REQUIRED)

# ソースファイル
set(SOURCES
    src/bindings/uvc_ext.cpp
    src/bindings/frame.cpp
)

# プラットフォーム固有のソースファイル
if(APPLE)
    list(APPEND SOURCES
        src/bindings/device_macos.mm
    )
elseif(WIN32)
    list(APPEND SOURCES
        src/bindings/device_windows.cpp
    )
elseif(UNIX)
    list(APPEND SOURCES
        src/bindings/device_linux.cpp
    )
endif()

# nanobind モジュール
nanobind_add_module(uvc_ext NB_STATIC FREE_THREADED ${SOURCES})

# インクルードディレクトリ
target_include_directories(uvc_ext PRIVATE src/bindings)

# プラットフォーム固有のライブラリ
if(APPLE)
    target_link_libraries(uvc_ext PRIVATE
        "-framework Foundation"
        "-framework AVFoundation"
        "-framework CoreMedia"
        "-framework CoreVideo"
    )
elseif(WIN32)
    target_link_libraries(uvc_ext PRIVATE
        mf
        mfplat
        mfuuid
        mfreadwrite
        ole32
    )
elseif(UNIX)
    # V4L2 は Linux カーネルの一部なのでリンク不要
endif()

# .pyi スタブファイルを生成
nanobind_add_stub(
    uvc_ext_stub
    MODULE uvc_ext
    OUTPUT ${CMAKE_CURRENT_BINARY_DIR}/uvc_ext.pyi
    PYTHON_PATH $<TARGET_FILE_DIR:uvc_ext>
    DEPENDS uvc_ext
    MARKER_FILE py.typed
)

# モジュールをインストール
install(TARGETS uvc_ext LIBRARY DESTINATION uvc)

# .pyi スタブファイルをインストール
install(FILES ${CMAKE_CURRENT_BINARY_DIR}/uvc_ext.pyi DESTINATION uvc)

# py.typed マーカーファイルをインストール
install(FILES ${CMAKE_CURRENT_BINARY_DIR}/py.typed DESTINATION uvc)
