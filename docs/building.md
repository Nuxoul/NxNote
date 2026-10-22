# 构建与发布

## 当前构建

需要 Rust MSVC 工具链、Visual Studio C++ Build Tools / Windows SDK 和 CMake。项目只有所选 A BlockEditor 路径，不再使用 `--features block-editor` 或 `--features notion-editor`。

```powershell
cargo build --locked --release
# 用户需要隔离数据时：
cargo run --locked -- --data-dir .\test-data
```

命令是使用说明，不代表本轮执行。当前默认数据为系统数据目录下 `NxNote`；`--hidden` 启动到托盘。不自动迁移 A/B 对比目录，也不自动注入示例。

`.cargo/config.toml` 对 Windows MSVC 使用静态 CRT；Release 开启优化、thin LTO、strip 和 abort panic。图标及 Kit 资源嵌入程序。

## 单文件打包要求

1. 用户明确请求打包后才进行 Release 编译；不顺带启动应用或执行测试。
2. 输出唯一命名的 Windows x64 EXE，复制时拒绝覆盖旧发布文件，不停止用户正在运行的程序。
3. 检查 PE x64 / Windows GUI、直接导入 DLL、SHA-256 和复制前后哈希。
4. 编译与文件检查不等于运行/IME/保存往返验收。

`dist/`、根目录 `*-windows-x64.zip` 和 target 为本地交付/构建输出，已忽略，不加入源码提交。旧文件保留，不因清理源码而删除。

## 当前单文件（2026-10-22）

- 文件：`dist/NxNote-v0.2.1-paragraph-spacing-20261022-144605-windows-x64.exe`
- 大小：22,930,432 字节（约 21.87 MiB）。
- SHA-256：`C697A5D2FBA1E3DBB59CE90967CACE2DE0215FA29CEE651D0D6B0BCD348E7384`
- Release 构建成功，耗时 1m58s，无编译 warning。x64 / PE32+ / Windows GUI、直接 DLL 依赖、无动态 CRT 和复制前后哈希检查通过。
- 分离“段内行高（自动折行）”与“段落间距（回车后）”；新增段落间距 0–48px、默认 0，保留旧行高值。段距变化会使块高度缓存失效，重新测量布局；插入符随行高的修改仍包含在内。
- 打包阶段未运行测试或程序。随后经用户明确授权，使用临时数据目录隔离运行：系统关闭设置 1 次、实际点击 X 8 次（含立即关闭 5 次）均保持主窗口运行、stderr 为空；取消按钮坐标未命中，不计入有效结果。未复现退出根因，用户随后反馈“好像好了”。未触碰正式笔记或用户实例，旧包保留。

## 上一版行高与插入符包（历史产物）

- 文件：`dist/NxNote-v0.2.1-lineheight-caret-20261022-143422-windows-x64.exe`
- 大小：22,927,872 字节（约 21.87 MiB）。
- SHA-256：`8C5E54967D2A85FB0ED00C9CB2EFA21ECD304EDB1E1C8D84E8087BBF68236091`
- Release 构建成功，耗时 2m04s，无编译 warning。x64 / PE32+ / Windows GUI 检查通过，无动态 CRT 直接导入；直接依赖均为 Windows 系统 DLL/API set，复制前后哈希一致。
- 在上一版基础上将最小行间距调为 0.5（默认仍 1.5），插入符高度等于实际行高并对齐行框顶部，绘制和输入法定位共用几何。
- 未执行测试、未启动程序或桌面自动化；实际排版、光标与设置关闭仍需运行验收。旧包保留，未停止用户进程、修改用户数据、提交或推送。

## 上一版紧凑 UI 包（历史产物）

- 文件：`dist/NxNote-v0.2.1-compact-ui-20261022-141734-windows-x64.exe`
- 大小：22,927,872 字节（约 21.87 MiB）。
- SHA-256：`52258022D67593781E7C5FF942FAD7D013B0669CE2CA5EA1B88BD7211A54EE0E`
- 首次编译发现下拉菜单包装不实现 Sizable、Base Tabs 不直接支持滚动锁；分别改为 Toolbar::content 与带 TabList 语义的原生滚动容器，保留 Base Tab。
- 修正后 Release 构建成功，耗时 1m46s，无编译 warning。x64 / PE32+ / Windows GUI 检查通过，无动态 CRT 直接导入；其他直接 DLL 均为 Windows 系统 DLL/API set，复制前后 SHA-256 一致。
- 包含紧凑单行文档栏、连续 UI/正文间距比例、可折叠格式工具、弱化 gutter/代码框和设置窗口复用/延后创建关闭处理。保留 A WYSIWYG、正式 NxNote 数据目录和原有字体/行高设置。
- 未执行测试、未启动程序或桌面自动化；设置关闭故障、布局与输入等仍待运行验收。新文件拒绝覆盖旧包，未停止用户进程或修改用户数据。

## 上一版主题包（历史产物）

- 文件：`dist/NxNote-v0.2.1-themes-20261022-125634-windows-x64.exe`
- 大小：22,892,032 字节。
- SHA-256：`821576BA913160CD33EC47BE72C46288E29BC14B060E97E66FA5CE0DDEBC72A8`
- 构建成功，耗时 2m06s；无编译错误，存在一条 gutter 未使用 `style` 参数的 warning。
- 已检查 x64 / Windows GUI、直接导入仅 Windows 系统 DLL、复制前后 SHA-256 一致。未覆盖旧包。
- 默认使用正式 `NxNote` 数据目录，或显式 `--data-dir` 指定的目录，不注入对比示例、不自动迁移 A/B 测试数据。
- 包含浅/深色全局配色、正文样式和选区重设计、恢复默认配色入口，以及此前行间距与行号修复。未执行测试、未启动程序或桌面自动化；实际颜色、选区与主题切换待用户验收。


## 现有 A 效果包（历史产物）

- 文件：`dist/NxNote-v0.2.1-A-block-editor-20261022-100120-windows-x64.exe`
- 大小：22,801,920 字节。
- SHA-256：`A2E42690C9F2EA6747CE711E940C5531E326C673EC8CA9DADF38F5C2EB106E9F`
- 该包此前实际通过 Release 编译和 PE / 系统 DLL 检查，但没有由代理启动或执行测试。
- 它仍是对比时的产物：默认使用 `NxNote-BlockEditor-Preview` 独立目录。当前源码与新包不会修改它。

## 本轮验证边界

已完成 Release 编译、单文件及 PE/DLL/哈希检查。未执行测试套件；用户授权后的设置关闭隔离复现范围见当前包记录，不代表全部交互或视觉验收。UI、设置生命周期、段内行高、段落间距及插入符改动作为本次源码提交交付；EXE、ZIP、临时诊断脚本和构建产物不提交到远端。
