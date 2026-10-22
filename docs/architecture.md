# 项目架构

NxNote 是 Rust 2021 / GPUI Kit 0.8.0 的 Windows 笔记应用。唯一笔记编辑器是用户选定的 `gpui-kit-markdown-editor` 0.1.0（此前对比包的 A 版）。没有 B 版 feature、源码 Editor fallback 或自制 Markdown 投影内核。

## 源码职责

| 模块 | 职责 |
| --- | --- |
| `main.rs` | 参数、初始化、Kit 根窗口和退出持久化 |
| `app.rs` | 应用动作、窗口/标签、对话框、编辑事件和延迟保存调度 |
| `note_editor.rs` | 上游 BlockEditor 适配、保存快照、格式工具栏、图片粘贴和块移动 |
| `session.rs` | 不依赖 UI 的当前笔记、标签顺序、应用关联、切换、捕获及脏状态 |
| `storage.rs` | 笔记/索引路径、读写、回收站、标题规则 |
| `fsutil.rs` | 原子文件写入 |
| `attachments.rs` | 图片外部存储及兼容的路径/文件名注册表 |
| `config.rs` | 配置兼容、独立整数字号和数据目录覆盖 |
| `settings_ui.rs` | Kit 设置窗口、暂存和验证 |
| `theme.rs` / `fonts.rs` | 主题、独立 UI/正文字体与字号、字体后台扫描 |
| `platform.rs` | Windows 窗口可见性/置顶、图片文件剪贴板兼容读取 |
| `hotkey.rs` / `tray.rs` / `watcher.rs` | 全局快捷键、托盘、前台应用观察和异步事件 |
| `autostart.rs` | Windows 开机启动设置 |

## 编辑和保存的数据流

```text
原 Markdown 文件 → Session.text → BlockEditor 导入
用户原生编辑 → 上游模型/事务/历史 → 提交快照 → InputEvent::Change
→ Session.edit → 延迟保存 / Ctrl+S → storage → 原子文件写入
```

- 上游拥有可编辑模型、选区、输入法、几何和撤销；应用不实现第二份输入内核。
- `original` 是载入原文，`baseline` 是初始模型导出的只读基准；`committed` 是供保存读取的快照，不是可编辑正文。只载入、聚焦和绘制不规范化原文件。
- 有效编辑后导出规范化 Markdown；模型回到初始基准时恢复原始载入文本。预编辑期间不推进提交快照。
- `Session` 负责保存失败保留脏状态、切换和恢复；读取失败不会以空正文覆盖原文件。删除进入 `.trash`。
- 图片异步保存后用上游 once-only 粘贴请求核验文档和选区；再检查焦点及预编辑，过期请求不插入。附件删除不与正文删除绑定。

## 数据和资源

默认使用 `dirs::data_dir()/NxNote`，`--data-dir` 可显式覆盖：

```text
config.toml
index.json
notes/<folder>/<note>.md
attachments/
attachments.json
notes/.trash/
```

不自动迁移对比包的 `NxNote-BlockEditor-Preview` 或 `NxNote-NotionEditor-Preview`。配置字段和既有目录键保持兼容。

`assets/icon.ico` 由 build.rs 嵌入，`assets/icon.png` 由托盘嵌入，二者保留。旧 Material Icons 字体没有调用方，已移除。

## 依赖边界

`gpui-kit =0.8.0`、`gpui-pre 0.3.9`、匹配的 HTTP client 使用同一运行时；不启用 Shell/WebView/Fast 或第二套 GPUI 后端。上游编辑器固定副本位于 `vendor/gpui-kit-markdown-editor`，许可证、设计和测试源码保留。最小补丁见其 `NXNOTE-UPSTREAM.md`；不把这些上游代码说成 NxNote 自研。

删除范围仅为失效实现、B 版、旧行编辑辅助、无引用资源及过时调研/自动化资料。没有重写 session/storage/config 格式，没有清空 target/dist 或用户数据。详见[编辑器契约](editor.md)与[构建发布](building.md)。
