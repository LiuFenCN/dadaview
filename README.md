# photoview

Windows 上基于 Rust + egui 的轻量图片查看器，同时能直接打开常见 3D 模型。

## 功能

- 图片浏览：缩放、拖动、文件夹内前后翻页（默认按修改时间新→旧，与资源管理器一致）
- 多窗口：每次打开都是独立窗口，互不影响；每个窗口可单独置顶
- 漫画模式：双页并排，翻页自动跳过非图片文件
- GIF 动图播放
- 3D 模型查看：拖动旋转、滚轮缩放；交互时软件光栅化快速预览，静止后用 GPU 渲染高清帧
- 3D 贴图：GLB/glTF 的 baseColorTexture 在两条渲染路径上都会显示；支持二进制 FBX（Blender / SDK 导出）

## 支持格式

- 图片：PNG、JPEG、BMP、WebP、TIFF、GIF、ICO
- 模型：OBJ、STL、GLB/GLTF/VRM、PLY、OFF、3DS、DAE、VTK、WRL、AMF、3MF、G-code、FBX、PMX/PMD、USD/USDZ
- 工程格式（Blend、STEP/IGES、3DM、C4D、MAX、SKP 等）会明确提示不支持

## 构建

需要 Rust（edition 2024）与 Windows 环境：

```bash
cargo build --release
```

产物为 `target/release/photoview.exe`。

## 测试

```bash
cargo test
```

用本机真实文件做手动回归：

```bash
PHOTOVIEW_FBX=模型.fbx cargo test -- --ignored load_real_fbx_sample
PHOTOVIEW_GLB=模型.glb cargo test -- --ignored load_real_glb_sample
```

## 许可

GNU Affero General Public License v3.0，见 [LICENSE](LICENSE)。

Copyright (c) 2026 cathi
