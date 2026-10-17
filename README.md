# DaDaView

**Windows 图片查看器 + 3D 模型预览** · A fast, lightweight image viewer with built-in 3D model preview for Windows.

基于 Rust + egui，单文件可执行，启动快、占用小；既能日常看图，也能直接打开 OBJ / STL / GLB / FBX 等常见模型。

[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL--3.0-blue.svg)](LICENSE)

## 功能 · Features

- 图片浏览：缩放、拖动、文件夹内前后翻页（默认按修改时间新→旧，与资源管理器一致）
- 多窗口：每次打开都是独立窗口，互不影响；每个窗口可单独置顶
- 漫画模式：双页并排，翻页自动跳过非图片文件
- GIF 动图播放
- 3D 模型查看：拖动旋转、滚轮缩放；交互时软件光栅化快速预览，静止后 GPU 渲染高清帧
- 3D 贴图：GLB/glTF 的 baseColorTexture 在两条渲染路径上都会显示；支持二进制 FBX（Blender / SDK 导出）
- Image browsing with zoom, pan and folder paging (newest-modified first, matching Explorer)
- Independent windows, per-window always-on-top, comic two-page mode, animated GIF
- 3D preview with software rasterization while orbiting and GPU rendering when idle
- glTF/GLB baseColor textures plus binary FBX support

## 下载 · Download

从 [Releases](https://github.com/LiuFenCN/dadaview/releases/latest) 下载：

- `DaDaView-*-x64-setup.msi` —— 安装版，默认装到 `C:\Program Files\DaDaView`，安装向导中可自定义安装目录（需要管理员权限）
- `DaDaView-*-x64-portable.zip` —— 免安装版，解压后直接运行 `dadaview.exe`，无需管理员权限

安装版会创建桌面和开始菜单快捷方式；安装位置在向导的“目标文件夹”页可浏览或直接输入，例如改到 `D:\Programs\DaDaView`。安装包未做代码签名，首次运行时 Windows SmartScreen 可能提示“未知发布者”，选择“更多信息 → 仍要运行”即可。

## 支持格式 · Supported formats

- 图片：PNG、JPEG、BMP、WebP、TIFF、GIF、ICO
- 模型：OBJ、STL、GLB/GLTF/VRM、PLY、OFF、3DS、DAE、VTK、WRL、AMF、3MF、G-code、FBX、PMX/PMD、USD/USDZ
- 工程格式（Blend、STEP/IGES、3DM、C4D、MAX、SKP 等）会明确提示不支持

## 构建 · Build

需要 Rust（edition 2024）与 Windows 环境：

```bash
cargo build --release
```

产物为 `target/release/dadaview.exe`。安装包用 WiX Toolset 3.11 生成，脚本在 [installer/dadaview.wxs](installer/dadaview.wxs)：

```bash
candle.exe -arch x64 -out installer/obj/dadaview.wixobj installer/dadaview.wxs
light.exe -ext WixUIExtension -cultures:zh-CN -out dist/DaDaView-0.1.0-x64-setup.msi installer/obj/dadaview.wixobj
```

## 测试 · Tests

```bash
cargo test
```

用本机真实文件做手动回归：

```bash
DADAVIEW_FBX=模型.fbx cargo test -- --ignored load_real_fbx_sample
DADAVIEW_GLB=模型.glb cargo test -- --ignored load_real_glb_sample
```

## 联系 · Contact

- 论坛 / Forum：<https://bbs.cathi.cn>
- QQ 群 / QQ Group：369748859
- 问题反馈 / Issues：<https://github.com/LiuFenCN/dadaview/issues>

## 许可 · License

GNU Affero General Public License v3.0，见 [LICENSE](LICENSE)。

Copyright (c) 2026 LiuFenCN
