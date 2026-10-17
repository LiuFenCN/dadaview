# 更新日志 · Changelog

本项目的所有重要变更记录。版本号遵循 `主.次.修订`。

## [未发布] - 2026-10-17

### 修复

- **「设为默认查看器」不再无效**（`src/file_assoc.rs`）
  - 根因：Win8 起，双击文件的默认打开方式由 `FileExts\.ext\UserChoice`（ProgId + 防篡改 Hash）决定，优先级高于 `HKCU\Software\Classes\.ext`。原实现只写 Classes / OpenWithProgids，从不写 UserChoice，导致按钮点击后毫无变化。
  - 修复：写入 UserChoice 时计算合法的 Hash（与 SetUserFTA / PS-SFTA 同一套算法：MD5 + 两轮 32 位混合，输入为 `扩展名 + 用户 SID + ProgId + 注册表键时间戳(分钟截断) + experience 字符串`）。写入后读回键的真实时间戳，跨分钟边界自动重试，保证哈希自洽。
  - 设置面板新增结果反馈：成功显示"已将 N 个格式的默认打开方式设为 DaDaView"，失败显示具体原因（此前失败只写 eprintln，界面上无任何提示）。

- **glTF 外部贴图缺失时整体加载失败**（`src/mesh3d.rs`）
  - 根因：部分导出的 .gltf 引用外部 PNG（如 `textures/` 目录），目录缺失时 gltf 库直接整体报错，拖累完好的几何数据也无法显示。
  - 修复：宽容导入——贴图缺失/解码失败时用 1×1 中性灰占位（材质退回 baseColorFactor 纯色）；buffer 缺失或短于声明长度时按声明补零（防切片越界）。加载成功但不完整时，3D 视图底部显示黄色提示"N 张贴图文件缺失，模型以无贴图模式显示"。
  - 实测：`dc_r18.gltf`（36997 顶点 / 53476 三角面 / 29 张贴图缺失）正常加载。

- **FBX 忽略 Model 变换与法线层**（`src/mesh3d.rs`）
  - 根因：FBX 的几何位于局部空间，真实位置由 Model 节点的 Lcl TRS 与父子层级决定；解析器此前完全丢弃这两项——非单位变换的 CAD / Mixamo / 旧版 Blender 导出文件会出现"所有部件叠在原点"；文件自带的法线层也被丢弃，硬表面棱角被平滑着色抹圆。
  - 修复：
    - TRS：从 Model 的 Properties70 **P 记录**取 Lcl Translation / Rotation / Scaling / RotationOrder，支持全部 6 种欧拉序（默认 XYZ）
    - 父子关系：解析 Connections 的 OO 记录（Blender 将 Model 平铺在 Objects 下，层级全靠连接描述），沿父链累乘世界矩阵（防环）
    - 法线：解析 LayerElementNormal，支持 ByPolygonVertex / ByVertice / ByPolygon 三种映射与 IndexToDirect 索引展开，经 inverse-transpose 法线矩阵跟随变换；无法线的几何保持原有索引式 + 平滑法线（省内存）
  - 实测：`dc_r18.fbx`（53476 三角形，与同模型 gltf 一致）、`66.fbx`（37758 三角形），均带 FBX 法线加载。

### 安装包

- 安装器源码从 WiX Toolset v3 迁移到 v5（`installer/dadaview.wxs`，由 `wix convert` 官方转换器生成）：
  - 顶层元素 `Product` → `Package`，`InstallScope`/`InstallPrivileges`/`Platform` 属性移除（由 `-arch x64` 与默认 perMachine 决定）
  - 标准目录改用 `StandardDirectory` 元素；`Component/@Win64` → `@Bitness`
  - UI 引用改为 `<ui:WixUI Id="WixUI_InstallDir" />`（Ui 扩展命名空间）
  - 构建方式改为 WiX v5 CLI（`dotnet tool install --global wix --version 5.0.2`）：`wix build -arch x64 -ext WixToolset.UI.wixext -b . -out dist/DaDaView-0.1.0-x64-setup.msi installer/dadaview.wxs`
  - MSI 通过 ICE 校验（仅 1 条提示性警告：AllowSameVersionUpgrades 允许同版本重装，属预期配置）
- 三件套产物（`dist/`）：`DaDaView-0.1.0-x64-setup.exe`（安装版启动器，内嵌 MSI 并自动请求管理员权限）、`DaDaView-0.1.0-x64-setup.msi`、`DaDaView-0.1.0-x64-portable.zip`（免安装版）。

### 测试

- 新增 UserChoice 哈希算法的真实向量单元测试（复现本机现存 Hash，逐位一致）
- 新增合成 FBX 测试：单 Model 变换精确落位（平移+旋转+缩放）、两级 Model 层级累加
- 新增"贴图缺失仍能加载"的 glTF 降级测试
- 全量单元测试通过（42 项）
