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
- `Ctrl+F`：查找当前文档全文；Enter /「下一个」跳到下一处。
- `Ctrl+,`：设置。
- `Ctrl+Tab` / `Ctrl+Shift+Tab`、`Ctrl+PageDown` / `Ctrl+PageUp`：切换标签。
- `Ctrl+1` 至 `Ctrl+9`：激活对应标签；`Ctrl+W`：关闭标签。
- 标签支持拖动排序：拖到目标标签左/右半边，在其前/后插入；保持当前笔记与撤销状态，顺序立即写入配置。
- `Alt+↑/↓`：移动行/选中块；`Alt+Shift+↑/↓`：复制；`Ctrl+Shift+K`：删除行/选中块。
- `Ctrl+=` / `Ctrl+-`：调整编辑字号。
- `Ctrl+\`：清除光标所在的最内层格式，或选区涉及的格式，保留文字；在格式起点 Backspace / 终点 Delete 可去掉对应标记，Ctrl+Z 恢复。
- 设置中的“界面字号”和“正文字号”分别控制 UI 和 Markdown 正文，均为整数 10–24 px，步长 1，无小数点。旧 `font_size` 继续作为正文字号，已有小数加载时四舍五入；左侧行号按原文编号。
- 编辑快捷键限定在文档上下文中；点击当前标签或正文会恢复文档焦点。`Ctrl+V` 支持截图位图及图片文件路径，读取失败会提示而非静默忽略。
- `Ctrl+Q`：保存并退出；普通关闭按配置隐藏到托盘。
- 默认全局显示/隐藏：`Ctrl+Alt+N`；默认剪贴板捕获：`Ctrl+Alt+Shift+C`。

热键被其他程序占用时会反馈错误。启动时捕获热键冲突不影响显示/隐藏；设置新热键失败时不保存并尝试恢复旧注册。

## GPUI Kit 组件

- 主窗口：`TitleBar`、`Toolbar`、`Button`、`DropdownMenu`、`TabBar` / `Tab`、`StatusBar`。
- 笔记编辑：单一 `NoteEditor` 文档输入控件，直接接收 GPUI 输入法事件；正文、选区、撤销与剪贴板不分段，不提供源码/图片编辑/预览模式。
- Markdown 排版：`markdown_projection.rs` 保存显示字符与原文位置映射，`document_layout.rs` 用同一字形布局完成文字绘制、鼠标命中、光标、选区和输入法候选框定位。图片标签继续组合 Kit `Button` / `HoverCard`。
- 快速切换：`Command` / `CommandState`。
- 新建/重命名/删除/标题规则：`Dialog`、`Input`、`DialogFooter`。
- 设置：自绘 `TitleBar`、`Form`、`Field`、`Switch`、`NumberInput`、`Combobox`、`Textarea`、`ColorPicker`、`TabBar`；字体列表在后台加载，不阻塞首开。
- 主题与提示：`Theme`、`Notification`。

遵循“自带组件 → 组件组合 → 必要业务扩展”的顺序。支持直接粘贴截图/图片文件，显示 `image.png ×` 标签，悬停预览；点击 × 只移除引用，不删除图片文件。全文复制是包含实际图片路径的纯文本，不复制位图。截图保存到数据目录的 `attachments/`，引用清单保存在 `attachments.json`。正文、选区、复制/剪切与撤销重做由文档层统一管理，不随自动就地编辑切换而分成多份。没有手动源码/预览切换入口。

此前修正版已按用户打包请求构建过 Release。本轮标签拖动、独立整数字号及参考项目驱动的 Markdown 修正仅做源码阅读和 Git 差异检查，未编译、未测试、未进行桌面自动化；已有 `dist/` EXE 不包含本轮改动。

列表与引用保留项目符号、缩进和引用栏；任务复选框可直接点击。表格直接在单元格内输入，Tab / Shift+Tab 切换单元格，Enter 移到下一行同列，末行继续可添加行，Shift+Enter 插入单元格换行。代码块保持代码排版，回车续缩进，不会把整块切换成源码输入框。跨段落拖选和 Shift 扩选使用文档级锚点，拖选边缘自动滚动；Ctrl+A / C 复制完整 Markdown 正文及实际图片路径。

Markdown 字号与行高统一以设置中的正文字号为基准，并使用 Kit 的 H1–H6 层级规则；粗体、斜体、链接不额外放大正文。格式移除通过原文中的成对标记修改，标题、强调、链接、行内代码、代码围栏、列表与引用均不需要切换到源码模式。

已实际浅克隆 MarkText/Muya 与 ProseMirror commands，核对标记显隐、删除和撤销规则。参考仓库固定提交、具体源码链接与采用/未采用规则见 [Markdown 编辑器参考](docs/markdown-editor-references.md)。标记内部现在只按字素删字符，格式内容边界退出格式与 `Ctrl+\` 清除格式仍为明确命令。

底层输入与布局另外实际克隆并参考 **Rust + GPUI 的 Zed**（不是所见即所得 Markdown 编辑器）。已按同栈接口修正原生粘贴/选区/文本长度、UTF-16 偏置裁剪、表格预编辑规范化和候选框定位。技术栈差异、具体 Rust 链接及排除的错误推断见 [Rust GPUI 参考](docs/rust-gpui-editor-references.md)。

## 模块

- `session.rs`：不依赖 UI 的笔记、标签、恢复、应用关联、捕获和保存状态。
- `app.rs`：GPUI 主窗口、动作与组件组合。
- `settings_ui.rs`：暂存设置，点击应用后验证并交给主窗口处理。
- `platform.rs` / `hotkey.rs` / `tray.rs` / `watcher.rs`：Windows 集成；后台事件通过异步通道进入 GPUI，即使窗口隐藏也能捕获、保存和退出。
- `editor_support.rs`：列表续行、图片 URL 包装与行编辑。
- `markdown_format.rs`：成对去格式、原文选区偏移映射与图片路径保护。
- `note_editor.rs`：统一文档输入法、正文选区、全文历史、导航、搜索与块内编辑。
- `markdown_projection.rs` / `document_layout.rs`：原文坐标映射、可编辑排版、真实字形命中与图片标签流式布局。
- `attachments.rs`：后台图片保存、路径引用恢复与文件名标签。

保存失败不清除脏状态；读取失败不会把原文件当作空笔记覆盖。删除进入 `.trash`，不直接销毁笔记。

官方文档：[GPUI Kit 中文组件目录](https://gpui-kit.com/zh-CN/component/)。
