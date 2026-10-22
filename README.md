# NxNote

Rust / GPUI Kit 构建的 Windows 轻量笔记应用。笔记正文使用用户选定的 **gpui-kit-markdown-editor（原 A 版）**，在渲染后的 Markdown 内容上直接编辑；不再包含 B 版、源码编辑器 fallback 或自制 Markdown 编辑内核。

## 功能

- 标题、段落、列表/待办、引用、表格、代码块和行内格式的原生块编辑。
- 格式工具栏、链接编辑、截图/图片文件粘贴；输入法、选区、事务和撤销由上游组件管理。
- Markdown 文件保存、延迟自动保存、多标签、标签拖动排序、笔记搜索切换。
- 独立界面/正文字体与整数字号（10–24 px）、Kit 设置窗口、主题。
- Windows 托盘、全局快捷键、剪贴板捕获、前台应用关联和开机启动设置。

## 构建与运行

需要 Rust MSVC 工具链、Visual Studio C++ Build Tools / Windows SDK 和 CMake。只有一个编辑器，无需 feature 开关。

```powershell
cargo build --locked --release
cargo run --locked -- --data-dir .\test-data
cargo run --locked -- --hidden
```

默认数据目录为系统数据目录下的 `NxNote`，保留既有 `config.toml`、`index.json`、`notes/<folder>/<note>.md`、附件及回收站格式。`--data-dir` 显式选择其他目录。不会自动迁移对比包数据或注入测试示例。

## 常用操作

| 操作 | 快捷键 |
| --- | --- |
| 保存 / 新建笔记 | Ctrl+S / Ctrl+N |
| 搜索切换笔记 / 设置 | Ctrl+P / Ctrl+, |
| 下一个 / 上一个标签 | Ctrl+Tab / Ctrl+Shift+Tab |
| 指定标签 / 关闭标签 | Ctrl+1…9 / Ctrl+W |
| 粗体 / 斜体 / 行内代码 | Ctrl+B / Ctrl+I / Ctrl+E |
| 编辑链接（留空移除） | Ctrl+K |
| 撤销 / 重做 | Ctrl+Z / Ctrl+Y |
| 列表缩进 / 表格导航 | Tab / Shift+Tab |
| 移动块 | Alt+↑ / Alt+↓ |
| 正文字号 | Ctrl+= / Ctrl+- |
| 保存退出 | Ctrl+Q |
| 全局显示/隐藏（默认） | Ctrl+Alt+N |
| 全局剪贴板捕获（默认） | Ctrl+Alt+Shift+C |

普通关闭按配置隐藏到托盘。旧复制/删除源码行命令已移除，不再占用快捷键；全文搜索没有另行接入。

## 保存边界

- 仅载入、聚焦和绘制不重写 Markdown；有效编辑后保存上游规范化 Markdown。**不承诺原文逐字节保真**，空白、列表松紧和标记形式可能变化。
- 预编辑不写入提交快照；保存失败保留脏状态，读取失败不以空正文覆盖原文件，删除进入 `.trash`。
- 图片原生渲染；不再提供旧文件名标签、悬停和 × 控件。截图保存到 `attachments/`，注册表 `attachments.json` 保持兼容，删除正文引用不会自动删除图片文件。
- 公式/Mermaid 图像渲染、Wiki/脚注导航及外部链接自动打开未接入。完整 Markdown 往返和 Windows IME 仍需用户验证。

## 项目结构

```text
src/                             NxNote 应用与存储、Windows 集成
vendor/gpui-kit-markdown-editor/  固定上游组件、许可证、测试与设计资料
assets/                          构建图标和托盘图标
docs/                            当前架构、编辑器契约和构建说明
.cargo/                          Windows 静态 CRT 配置
```

- [项目架构](docs/architecture.md)
- [编辑器契约与限制](docs/editor.md)
- [构建与单文件发布](docs/building.md)
- [上游来源及本地补丁](vendor/gpui-kit-markdown-editor/NXNOTE-UPSTREAM.md)

GPUI Kit 固定 0.8.0、gpui-pre 0.3.9；编辑组件固定上游提交 `31608fa74bdbeca79d9901bf2daa48ea7ecf1ecc`（0.1.0、Apache-2.0）。

## 验证状态

清理后的 A 默认编辑器已按用户请求通过 Windows x64 Release 编译，生成 `dist/NxNote-v0.2.1-block-editor-20261022-103726-windows-x64.exe`。已检查 PE 架构、系统 DLL 导入和复制哈希，详见[构建发布](docs/building.md)。**未执行测试、未启动程序或进行桌面自动化**，编译不等于交互验收。新包默认使用正式 `NxNote` 数据目录；旧包、ZIP、用户笔记及配置未改动。
