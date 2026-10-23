# 更新日志 · Changelog

本项目的所有重要变更记录。版本号遵循 `主.次.修订`。

## [未发布] - 2026-10-23

### 新增

- **每格式专属文档图标**（此前所有关联格式在无缩略图时一律显示软件图标，无法区分文件类型）
  - `assets/ext-icons/`：33 个格式文档图标（`gen_icons.py` 程序化生成，4x 超采样）——白色文档纸 + 类别色折角 + 色带上印格式名；粉=图像、蓝=3D 模型、绿=3D 打印
  - 嵌入 dadaview.exe（资源 ID 2 起），`DefaultIcon` 按 Explorer 图标序号指向对应图标；应用图标（快捷方式/任务栏）不受影响
  - 文件关联改为**每格式一个 ProgID**（`DaDaView.<ext>`），各自的 FriendlyTypeName 带格式名（如「PNG 图像 文件 (DaDaView)」）
  - 清单唯一权威定义在 `src/ext_icons.rs`（`SUPPORTED_EXTS` 与文档图标清单同源；新增格式 = 改一处 + 重跑 `gen_icons.py` + 重新嵌入）
  - 程序启动时静默迁移仍指向旧通用 ProgID（`DaDaView.Image`）的关联——只改写仍是我们旧值的键，用户后来手动改成其它程序的关联不动，幂等可重复
  - 注意：文档图标只对「DaDaView 为默认打开方式」的格式生效；有缩略图时资源管理器仍显示缩略图

### 修复

- **构建流水线漏构建 embed-icon 自身**：`--doc-icons` 传给旧版 embed-icon 时会把 `--doc-icons` 当 exe 路径、把 dadaview.exe 当 ICO 解析，报「不是合法的 ICO 文件」且极具迷惑性。README 构建顺序已补上 `cargo build --release --bin embed-icon` 第 0 步

## [未发布] - 2026-10-19

### 新增

- **许可协议中文化**（`LICENSE.zh-CN.md`）
  - `LICENSE` 保留 GNU AGPL v3 英文官方原文（具法律效力）；新增中文要点导读，覆盖：四条自由、六项义务、第 13 条网络服务条款（AGPL 与 GPL 的关键差异）、担保免责、分发前检查清单
  - 安装向导许可页改为「中文导读 → 分隔线 → 英文原文」，并在导读开头明确声明仅供参考、歧义以英文为准
  - `LICENSE.zh-CN.md` 一并进入安装包 payload 与免安装版压缩包

- **安装器支持升级 / 修复检测**（`installer/launcher/src/main.rs`）
  - 新增 `detect_installed()`：读 `HKLM\...\Uninstall\DaDaView` 取版本与安装位置；再枚举 `Uninstall` 根键，通过 `DisplayName == DaDaView` + `UninstallString` 含 `msiexec`（或子键名形如 `{GUID}`）识别旧 MSI 版遗留条目
  - 欢迎页据此显示：已安装版本、原安装位置、本次属于「升级安装 / 修复安装（覆盖同版本）/ 降级安装」（版本按点分数字段逐段比较）
  - 「选择安装目录」页把默认值预填为原安装位置，避免同一程序装出两份
  - 检测到旧 MSI 记录时，目录页多出勾选项「清理旧的 MSI 安装记录」——只删注册表键，**不动任何已安装文件**，也不调用 msiexec（规避 2502/2503）。共清理三处：`Uninstall\{GUID}`（「应用和功能」列表里的重复项来源）、`Installer\UserData\<SID>\Products\<packed>`（MSI 每用户安装属性，含 `LocalPackage`）、`Classes\Installer\Products\<packed>`（MSI 产品注册）。只删前两处会让 MSI 内部仍认为产品已安装
  - `--quiet` 静默安装同样沿用原安装位置并自动清理 MSI 记录
  - 写入「应用和功能」条目时补上 `InstallDate`（YYYYMMDD），列表里的安装日期不再靠系统猜测
  - 覆盖正在运行的 exe 时给出「请先退出 DaDaView」的明确提示，而非裸的「拒绝访问」

### 修复

- **exe 完全没有版本信息资源**
  - 现象：右键 `dadaview.exe` / `setup.exe` → 属性 → 详细信息，文件版本、产品版本、产品名称、版权全部为空
  - 修复：`src/bin/embed-icon.rs` 扩展为同时写 `RT_VERSION`，手写 `VS_VERSIONINFO` 结构（`VS_FIXEDFILEINFO` + `StringFileInfo\040904B0` + `VarFileInfo\Translation`），所有子块按 DWORD 对齐
  - 坑：`VS_VERSIONINFO` 的 padding 必须**相对整个块起始**计算（块头 6 字节也要算进去），只按 body 长度对齐会让 Windows 完全读不到版本资源——写入成功但属性页依然空白
  - 版本号从根 `Cargo.toml` 读取，三个 exe（主程序 / 卸载器 / 安装器）统一

- **版本号不再需要改多处**
  - `installer/launcher/build.rs` 读根 `Cargo.toml` 的 `[package] version`，通过 `cargo:rustc-env=DADAVIEW_VERSION` 注入，安装器的 `APP_VERSION` 改用 `env!("DADAVIEW_VERSION")`
  - 实测：把根 `Cargo.toml` 改成 0.9.9 → 构建输出立刻显示「DaDaView 版本号：0.9.9」，改回即恢复

## [未发布] - 2026-10-18

### 新增

- **软件图标**（`assets/dadaview.ico`、`assets/dadaview-512.png`）
  -  mascot 插画（粉毛猫娘 + 放大镜看方块猫）制成多尺寸图标：16 / 24 / 32 / 48 / 64 / 128 / 256 px，PNG 压缩条目，Lanczos 缩放 + USM 锐化；512×512 母图另行保存（ICO 规范的目录项宽高各 1 字节，单帧上限即 256）
  - **exe 图标资源**：新增 `src/bin/embed-icon.rs` 工具，用 kernel32 的 `BeginUpdateResourceW / UpdateResourceW / EndUpdateResourceW` 把 ico 写入 PE 的 RT_ICON / RT_GROUP_ICON，不依赖 rc.exe / windres / winres。资源管理器缩略图、任务栏、`DefaultIcon` 注册表（`file_assoc.rs` 早已指向 `exe,0`）随即生效
  - **窗口图标**：`src/main.rs` 编译期内嵌 512 PNG（`include_bytes!`），经 `ViewportBuilder::with_icon` 设置——免安装版不附带 assets 目录也有标题栏/任务栏图标
  - **安装包图标**：`installer/dadaview.wxs` 增加 `<Icon Id="DaDaViewIcon">` + `ARPPRODUCTICON`，「应用和功能」与安装向导显示应用图标；安装版启动器（setup.exe）同样嵌入图标
  - 验证：`embed-icon --verify` 对三个 exe 逐一确认图标组可加载；MSI 经 ICE 校验（仅预期内 ICE61 提示）；解包 MSI 比对 SHA256，内置 exe 与带图标的本地构建逐字节一致

### 文档

- README 构建章节更新为 WiX v5 CLI 与图标嵌入的完整流程（原文停留在 WiX v3 的 candle/light 命令）

### 安装器

- **安装器从 WiX/MSI 改为纯 exe 方案**（`installer/launcher` + 新增 `installer/uninstaller`）
  - 背景：部分系统运行 MSI 报 2503/2502（Windows Installer 组件错误），且 MSI 快捷方式图标、目标指向连续踩坑（通告式快捷方式不读 exe 图标资源；非通告式不写 Target 时默认指向安装目录）。纯 exe 方案不调用 msiexec，彻底规避该类错误
  - 安装器流程：UAC 提权（未提权则经 ShellExecuteExW runas 重启自身）→ IFileDialog 选择安装目录（默认 `C:\Program Files\DaDaView`，`--quiet` 跳过对话框）→ 释放 payload → IShellLinkW 创建开始菜单/桌面快捷方式（IconLocation 指向 exe，即用 exe 内嵌图标）→ 写 HKLM 卸载注册表项（DisplayName/DisplayIcon/UninstallString/EstimatedSize 等）
  - payload（dadaview.exe + README + LICENSE + 卸载器）由 `build.rs` 构建期打成 deflate zip 内嵌，安装时解压；安装器 6.6MB（原始 payload 14.6MB）
  - 卸载器（211KB）：删除快捷方式与注册表项后，经隐藏 cmd 延迟 rmdir 移除安装目录（含自身）
  - windows 0.58 API 坑：注册表函数返回 `WIN32_ERROR` 新建结构体（`.0` 取 u32，无 From 转换，需自行转 `Error::from_hresult(HRESULT::from_win32)`）；`IPersistFile` 在 `Win32::System::Com`；接口转换 `.cast()` 需 `use windows::core::Interface`；IShellLinkW 的方法为 PCWSTR 泛型（ANSI 版才是 PCSTR）
  - 验证：payload 解压单元测试（与源构建逐字节一致）；从成品 setup.exe 中挖出内嵌 zip 核对 4 个文件 SHA256 全部匹配；图标 `--verify` 通过
  - 不再分发 `setup.msi`；`installer/dadaview.wxs` 停用保留

- **安装器自带原生向导界面**（`installer/launcher/src/main.rs`）
  - 背景：去掉 MSI 后，双击 setup.exe 只剩一个「选择目录」对话框，缺少许可协议、进度反馈与完成页
  - 实现：手写 Win32 窗口（不引入 egui/GUI 依赖），五页向导：欢迎 → 许可协议（内嵌 LICENSE，必须勾选接受才能「下一步」）→ 选择安装目录（可浏览更改，含所需空间与两个快捷方式勾选项）→ 安装进度（进度条 + 当前文件 + 百分比）→ 完成（可勾选立即运行）
  - 经典布局：顶部白色页头带标题与当前步骤名，底部「上一步 / 下一步 / 取消」按钮与分割线；随页面切换按钮文案与可用状态自动变化
  - 进度刷新：安装动作放后台线程，主线程用消息泵保持界面响应，逐文件更新进度；安装期间禁用所有按钮并拦截关闭，避免半途退出
  - DPI：进程声明 Per-Monitor V2 感知，按系统 DPI 缩放窗口尺寸与字体，高分屏不模糊
  - windows 0.58 补充坑：`EnableWindow` / `SetFocus` 在 `Win32::UI::Input::KeyboardAndMouse`，而 `UpdateWindow` / `InvalidateRect` 在 `Win32::Graphics::Gdi`（并非全在 `WindowsAndMessaging`）；`CreateWindowExW` 返回 `Result<HWND>` 而非裸句柄；句柄参数不接受 `Option<HWND>`，要传 `HWND::default()`；`GetModuleHandleW` 返回 `HMODULE`，需转成 `HINSTANCE`

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
