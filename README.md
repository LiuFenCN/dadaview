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

- `DaDaView-*-x64-setup.exe` —— 安装版（推荐）：双击后自动请求管理员权限，随后进入安装向导：欢迎 → 许可协议 → 选择安装目录（默认 `C:\Program Files\DaDaView`，可浏览更改）→ 安装进度 → 完成。安装器为纯 exe（自释放 payload，不走 Windows Installer / msiexec），会创建桌面与开始菜单快捷方式、写入「应用和功能」卸载项（含独立卸载程序）。支持 `--quiet` 静默安装
- `DaDaView-*-x64-portable.zip` —— 免安装版，解压后直接运行 `dadaview.exe`，无需管理员权限

安装位置在向导的「选择安装目录」页中指定，例如改到 `D:\Programs\DaDaView`。安装包未做代码签名，首次运行时 Windows SmartScreen 可能提示“未知发布者”，选择“更多信息 → 仍要运行”即可。

**升级与修复**：安装器启动时会先读取注册表，检测本机是否已有安装。若已安装，欢迎页会显示当前版本、原安装位置以及本次是「升级 / 修复（同版本覆盖）/ 降级」，并把安装目录预填为原位置，避免装出两份。若检测到旧 MSI 版遗留的安装记录，会提供勾选项将其一并清理（只删「应用和功能」里的记录，不动任何已安装文件）。

**版本号**：只维护根 `Cargo.toml` 的 `version` 一处。安装器的版本号、`exe` 属性页里的「文件版本 / 产品版本」、以及「应用和功能」里显示的版本都会自动跟随。

**许可协议**：`LICENSE` 为 GNU AGPL v3 官方英文原文（具法律效力）；`LICENSE.zh-CN.md` 是中文要点导读，仅供参考。安装向导的许可页会先显示中文导读、再显示英文原文。

## 支持格式 · Supported formats

- 图片：PNG、JPEG、BMP、WebP、TIFF、GIF、ICO
- 模型：OBJ、STL、GLB/GLTF/VRM、PLY、OFF、3DS、DAE、VTK、WRL、AMF、3MF、G-code、FBX、PMX/PMD、USD/USDZ
- 工程格式（Blend、STEP/IGES、3DM、C4D、MAX、SKP 等）会明确提示不支持

## 构建 · Build

需要 Rust（edition 2024）与 Windows 环境：

```bash
cargo build --release
```

产物为 `target/release/dadaview.exe`。图标资源与版本信息（VERSIONINFO）都不进二进制源码，由 `embed-icon` 工具在编译后写入 exe（走 kernel32 资源更新 API，无需 rc.exe / windres / winres）：

```bash
cargo build --release --bin embed-icon                                 # embed-icon 自身也要最新
./target/release/embed-icon.exe --doc-icons target/release/dadaview.exe  # 应用图标 + 各格式文档图标 + 版本信息
./target/release/embed-icon.exe --verify target/release/dadaview.exe     # 验证图标资源可加载
```

图标分两层，均嵌在 dadaview.exe 内：

- **应用图标**（`assets/dadaview.ico`，资源 ID 1）——快捷方式 / 任务栏用
- **格式文档图标**（`assets/ext-icons/<ext>.ico`，33 个，由 `gen_icons.py` 生成，资源 ID 2 起）——文件**无缩略图时**的兜底图标，白纸 + 折角 + 色带上印格式名（PNG / GLB / OBJ…），粉=图像、蓝=模型、绿=3D 打印

文件关联随之按**每格式一个 ProgID**（`DaDaView.<ext>`）注册，`DefaultIcon` 指向各自的文档图标序号；清单一律以 `src/ext_icons.rs` 为准（`SUPPORTED_EXTS` 与 `DOC_ICON_EXTS` 同源，改格式只动一处 + 重跑 `gen_icons.py`）。程序启动时会静默迁移仍指向旧通用 ProgID（`DaDaView.Image`）的关联，用户无需手动重设。注意：文档图标只对「DaDaView 是默认打开方式」的格式生效；有缩略图时资源管理器仍显示缩略图。

版本号由 `embed-icon` 从根 `Cargo.toml` 读取，因此升版本只需改那一处。手写 `VS_VERSIONINFO` 时注意：所有子块的 **padding 是相对整个块起始**（含 6 字节块头）对齐到 DWORD，漏算块头会让 Windows 完全读不到版本资源。

窗口标题栏 / 任务栏图标则编译期内嵌（`src/main.rs` 的 `include_bytes!("../assets/dadaview-512.png")`），免安装版无需附带 assets 目录。

安装器为纯 exe 方案（源码 `installer/launcher` + `installer/uninstaller`），不依赖 Windows Installer——部分系统上 msiexec 会报 2502/2503，纯 exe 彻底规避该类错误。

安装器自带 Win32 原生向导（无额外 GUI 依赖）：欢迎 → 许可协议（中文导读 + `LICENSE` 英文原文，需勾选接受）→ 选择安装目录（可浏览更改，显示所需空间）→ 安装进度（进度条 + 当前文件 + 百分比）→ 完成（可勾选立即运行）。安装动作在后台线程执行，主线程靠消息泵保持界面刷新；安装期间禁用按钮并拦截关闭，避免半途退出。进程声明 Per-Monitor V2 DPI 感知，按系统 DPI 缩放窗口与字体。

安装前会调用 `detect_installed()` 扫描 `Uninstall` 根键：既认本安装器写入的 `DaDaView` 条目，也认旧 MSI 版留下的 `{GUID}` 条目（`DisplayName == DaDaView` 且 `UninstallString` 含 `msiexec`），后者可在向导中一键清除注册表记录。

安装器直接释放 payload、创建快捷方式（IShellLink）并写入「应用和功能」注册表项；payload 由 `build.rs` 在构建期打成 deflate zip 内嵌（`OUT_DIR/payload.zip`），因此**构建顺序不能乱**——主程序与卸载器必须先构建：

```bash
# 0. embed-icon（先构建自身，避免把 --doc-icons 当路径解析）
cargo build --release --bin embed-icon

# 1. 卸载器（会被安装器复制到安装目录，由「应用和功能」调用）
cargo build --release --manifest-path installer/uninstaller/Cargo.toml

# 2. 主程序 + 图标（--doc-icons 嵌入 33 个格式文档图标）
cargo build --release
./target/release/embed-icon.exe --doc-icons target/release/dadaview.exe

# 3. 安装器（build.rs 自动打包 payload.zip 并内嵌）
cargo build --release --manifest-path installer/launcher/Cargo.toml
./target/release/embed-icon.exe installer/launcher/target/release/dadaview-setup.exe
cp installer/launcher/target/release/dadaview-setup.exe dist/DaDaView-0.1.0-x64-setup.exe
```

卸载流程：`dadaview-uninstall.exe` 删除快捷方式、卸载注册表项后，以隐藏 cmd 延迟移除整个安装目录。安装器的 payload 解压完整性有单元测试保障（`cargo test --manifest-path installer/launcher/Cargo.toml`）。

> 历史上的 WiX/MSI 管线（`installer/dadaview.wxs`）已停用，文件保留作参考；重新启用需自行恢复 README 旧版说明。

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
