# NxNote

基于 Rust / GPUI Kit 的 Windows 轻量笔记应用。界面全面使用 GPUI Kit 0.7.3，不再依赖 egui / eframe。

## 构建与运行

需要 Rust MSVC 工具链、Visual Studio C++ Build Tools / Windows SDK 和 CMake。

```powershell
cargo run --locked
cargo run --locked -- --data-dir .\test-data
cargo run --locked -- --hidden
cargo build --locked --release
cargo test --locked
```

`--data-dir` 用于便携运行或隔离测试；不传时继续使用系统数据目录下的 `NxNote`。`--hidden` 启动到托盘。数据仍是 `config.toml`、`index.json` 和 `notes/<应用键>/<笔记名>.md`，保留旧版配置、应用目录键及回收目录。

## 操作

- `Ctrl+S`：保存；停止输入后自动延迟保存。
- `Ctrl+N`：新建当前应用/全局文件夹下的笔记。
- `Ctrl+P`：搜索并切换所有应用/笔记。
- `Ctrl+F`：使用 Kit 编辑器内置搜索。
- `Ctrl+Shift+P`：切换 Markdown 源码编辑与预览。
- `Ctrl+,`：设置。
- `Ctrl+Tab` / `Ctrl+Shift+Tab`、`Ctrl+PageDown` / `Ctrl+PageUp`：切换标签。
- `Ctrl+1` 至 `Ctrl+9`：激活对应标签；`Ctrl+W`：关闭标签。
- `Alt+↑/↓`：移动行/选中块；`Alt+Shift+↑/↓`：复制；`Ctrl+Shift+K`：删除行/选中块。
- `Ctrl+=` / `Ctrl+-`：调整编辑字号。
- `Ctrl+Q`：保存并退出；普通关闭按配置隐藏到托盘。
- 默认全局显示/隐藏：`Ctrl+Alt+N`；默认剪贴板捕获：`Ctrl+Alt+Shift+C`。

热键被其他程序占用时会反馈错误。启动时捕获热键冲突不影响显示/隐藏；设置新热键失败时不保存并尝试恢复旧注册。

## GPUI Kit 组件

- 主窗口：`TitleBar`、`Toolbar`、`Button`、`DropdownMenu`、`TabBar` / `Tab`、`StatusBar`。
- 笔记编辑：`Editor` / `EditorState`，复用语法高亮、行号、搜索、选区和撤销。
- Markdown 预览：Kit 的 `TextView`，通过内置 HTTP 客户端加载远程图片。
- 快速切换：`Command` / `CommandState`。
- 新建/重命名/删除/标题规则：`Dialog`、`Input`、`DialogFooter`。
- 设置：`Form`、`Field`、`Switch`、`NumberInput`、`Combobox`、`Textarea`、`ColorPicker`、`TabBar`。
- 主题与提示：`Theme`、`Notification`。

遵循“自带组件 → 组件组合 → 必要业务扩展”的顺序。编辑体验采用源码编辑 + 预览切换，不再使用旧版隐藏 Markdown 标记和覆盖绘制的 Typora 式行内编辑。快捷键设置使用文本输入并校验，而不是旧版按键录制。

## 模块

- `session.rs`：不依赖 UI 的笔记、标签、恢复、应用关联、捕获和保存状态。
- `app.rs`：GPUI 主窗口、动作与组件组合。
- `settings_ui.rs`：暂存设置，点击应用后验证并交给主窗口处理。
- `platform.rs` / `hotkey.rs` / `tray.rs` / `watcher.rs`：Windows 集成；后台事件通过异步通道进入 GPUI，即使窗口隐藏也能捕获、保存和退出。
- `editor_support.rs`：列表续行、图片 URL 包装与行编辑；文本修改和撤销仍由 Kit 管理。

保存失败不清除脏状态；读取失败不会把原文件当作空笔记覆盖。删除进入 `.trash`，不直接销毁笔记。

官方文档：[GPUI Kit 中文组件目录](https://gpui-kit.com/zh-CN/component/)。
