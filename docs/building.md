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

本轮完成 Release 编译、单文件及文件检查；未执行测试、启动、提交或推送。此前源码已在 95ef0a0 推送，后续行号、行间距及主题改动尚未提交。EXE、ZIP 和构建产物不提交到远端。
