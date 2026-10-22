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

- 文件：`dist/NxNote-v0.2.1-block-editor-20261022-103726-windows-x64.exe`
- 大小：22,796,288 字节。
- SHA-256：`17997EF88BA6FD01AABB4B1EBE93C3D173E508EA64A50ADA117D9359E8572719`
- 构建：`cargo build --locked --offline --release --target-dir target\single-file-fixes` 成功，耗时 3m41s，无编译错误或 warning。
- 已检查 x64 / Windows GUI、直接导入仅 Windows 系统 DLL、复制前后 SHA-256 一致。未覆盖旧包。
- 默认使用正式 `NxNote` 数据目录，或显式 `--data-dir` 指定的目录，不注入对比示例、不自动迁移 A/B 测试数据。
- 未执行测试、未启动程序或进行桌面自动化；Windows IME、Markdown 往返与实际交互仍待用户验收。


## 现有 A 效果包（历史产物）

- 文件：`dist/NxNote-v0.2.1-A-block-editor-20261022-100120-windows-x64.exe`
- 大小：22,801,920 字节。
- SHA-256：`A2E42690C9F2EA6747CE711E940C5531E326C673EC8CA9DADF38F5C2EB106E9F`
- 该包此前实际通过 Release 编译和 PE / 系统 DLL 检查，但没有由代理启动或执行测试。
- 它仍是对比时的产物：默认使用 `NxNote-BlockEditor-Preview` 独立目录。当前源码清理不会修改它，也不表示已有正式目录版新包。

## 本轮验证边界

项目清理阶段完成源码语法、Cargo metadata/依赖图和文件引用检查；随后按本次请求完成上述 Release 编译、单文件及文件检查，并准备提交源码文档、正常推送 main。A 上游测试源码与现有有效应用测试保留但未执行；EXE、ZIP 和构建产物不提交到远端。
