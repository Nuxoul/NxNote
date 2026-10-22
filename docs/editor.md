# Markdown 编辑器契约

## 选型与来源

唯一笔记编辑器为 [wabzqem/gpui-kit-markdown-editor](https://github.com/wabzqem/gpui-kit-markdown-editor)，固定提交 [`31608fa74bdbeca79d9901bf2daa48ea7ecf1ecc`](https://github.com/wabzqem/gpui-kit-markdown-editor/tree/31608fa74bdbeca79d9901bf2daa48ea7ecf1ecc)，版本 0.1.0、Apache-2.0。依赖 Kit 0.8.0 / gpui-pre 0.3.9。

这是第三方渲染式编辑组件，使用 Kit 的 `base::text::editor_host` 渲染节点和收集几何，并由自己的 BlockEditorState 维护原生输入与块模型。它不是官方 `component::input::Editor` 的配置模式；后者是源码编辑器。输入、绘制、导出证据可查上游 `src/state.rs`、`src/element.rs`、`src/direct.rs` 和 `src/serialize.rs`。

## 接入范围

- `BlockEditorState` / `BlockEditor` 是唯一可编辑状态/视图。
- 原生标题、段落、列表/待办、引用、表格、代码块和行内格式；保留上游跨块选区、事务、IME 和撤销重做。
- NxNote 仅提供保存快照、可折叠的 Kit 格式按钮（文档栏 Aa / 笔记菜单展开，默认收起）、Ctrl+K 链接弹窗、图片存储和原生块移动命令。
- 快捷键：Ctrl+B/I/E 格式，Ctrl+K 链接，Ctrl+Z/Y 撤销/重做，Tab/Shift+Tab 列表缩进/表格导航；Alt+↑/↓ 调用上游 MoveBlocks。
- 旧源码行移动/复制/删除、模拟源码选区 setter 和多版本标题接口已清理，不再挂载只会提示“不支持”的快捷键。
- 正文左侧显示逻辑文本行号，回车递增，自动折行不重复编号；表格同一行共用编号。行号栏固定总宽 64px（含 16px 手柄区），不随 9→10、99→100 等行数位数变化而移动正文；独立正体等宽字体、右对齐、背景贴编辑区左侧并覆盖上下留白，右侧保留细分隔线。当前行只提亮数字，不画方块；纵坐标沿用原生排版，不改正文段落间距。
- 界面不再显示顶部“Markdown 块编辑·保存会规范化 Markdown”说明，也不在底部字数后重复显示编辑器类型；规范化保存的限制保留在本文，而非占用正文空间。
- 设置 → 外观 → **段内行高（自动折行）**：原 `line_height`，0.5–2.5 倍、步长 0.1、默认 1.5，控制一段文字内部的行框高度，不控制回车后两个段落之间的额外空白。已有值保留，低于 1.0 可能让字形重叠；不要靠降低它来消除段距。插入符高度仍等于所在行高，输入法定位共用几何。
- **段落间距（回车后）**：新增 `paragraph_spacing`，0–48 逻辑像素、步长 1、默认 0；控制普通 Markdown 段/块之间额外的留白（用户截图红框处），独立于行高、字号和 UI 缩放。旧配置缺省为 0，点击应用才保存。段距变化加入高度缓存失效条件，重测后更新行号、插入符和滚动几何，不重建文档/历史。设为 0 只移除额外留白，不删除真实空段，也不取消标题/列表/代码自身的内部排版空间。
- 保留原有字体配置及 Consolas 默认值，不重置用户字体或行高。代码横向/底部内边距仍随正文按 12px/16px 比例缩放，去掉外框，顶部保留 Kit 语言标签空间。空段由上游模型处理，不承诺源码空行逐字保真。
- 浅色/深色界面、正文与原生选区共用统一色板；文本使用实色选区背景，图片原子采用透明叠色，避免遮住内容。完整色值、旧默认配置迁移及检查边界见[主题配色](themes.md)。
- `theme::LayoutMetrics` 分离界面与正文比例：界面尺寸按 UI 字号/16 连续缩放，控件高基准 24px（最低 22px）、文档栏高 28px（最低 24px）；不再按字号跳变 Small/Medium/Large。正文上下留白基准 8px、右侧 12px，随正文字号缩放。gutter 固定 64px、正文同底色、细分隔线保留，当前行只提亮数字。
- 原操作栏与标签栏合并为一条文档栏：带 TabList 语义的原生滚动容器与 Kit Base Tab 提供标签语义，保留鼠标滚轮转换、全部标签菜单、选中标签按需显露；原关闭、拖拽排序、Ctrl+Tab/数字键等动作保留。窄窗口将钉住和格式操作收进笔记菜单，设置和全部标签入口保留。基础 Tab 不另行提供箭头键 roving focus，不将其宣称为新增功能。

## 保存与图片

初始原文与模型导出基准分开记录，打开/聚焦不触发重写。真实编辑后保存上游 canonical Markdown，不保证源码逐字节保真；回到初始模型时恢复载入原文。保存快照不包含活动 IME 预编辑。存储失败处理仍由 Session 承担。

图片通过上游 Host clipboard policy 接入：普通文字由组件继续处理，截图/图片文件在后台保存或注册。完成后验证 once-only 请求、文档/选区、焦点和预编辑状态，再通过现有模型插入 Markdown 图片。图片使用原生渲染，不是旧文件名 chip、HoverCard 或 × 控件。附件物理文件不随引用删除而删除；旧普通路径不自动转换为图片语法。

`attachments.json` 格式保留兼容；旧“全文扫描路径→标签”代码无调用方，已移除，注册和文件保存仍保留。

## 明确限制

- 规范化可能改变空白、列表松紧和标记形式；扩展语法、空段和完整往返需用户验证，不承诺原文无损。
- 上游主要在 Linux 验证，Windows/macOS IME 与键位虽有实现但没有完整运行验收。用户选择 A 是选型确认，不代表全部交互测试通过。
- 全文搜索、外部链接自动打开、公式/Mermaid 图像渲染、Wiki/脚注导航及旧自制格式清除没有另行接入。
- 当前仅启用已有 Markdown tree-sitter features，不为效果对比拉入所有代码语言语法；非 Markdown 代码高亮范围受实际依赖特性限制。
- 相对图片路径等资源策略沿用组件现状，不宣称已补齐笔记文件夹基准解析。

## 上游维护

[本地修改记录](../vendor/gpui-kit-markdown-editor/NXNOTE-UPSTREAM.md) 包含稳定 Rust Fenwick low-bit 兼容和受控图片 Markdown 粘贴接口。保留上游测试源码、fixture、设计文档及许可证；本轮未执行它们。升级应固定新提交、先核对模型和 host API，不直接跟踪移动分支。

## Zed 布局参考

核对的官方源码固定在 `130f1e92a48ecdcc210419c5d9a52d605fa18fc0`：

- [Tab 尺寸及状态边框](https://github.com/zed-industries/zed/blob/130f1e92a48ecdcc210419c5d9a52d605fa18fc0/crates/ui/src/components/tab.rs)：`content_height` / `container_height` 使用统一 DynamicSpacing，而非每个区域任意增加高度。
- [编辑器行号布局](https://github.com/zed-industries/zed/blob/130f1e92a48ecdcc210419c5d9a52d605fa18fc0/crates/editor/src/element.rs#L2857)：使用显示行、行高、滚动偏移和 gutter 右边距定位，并独立决定当前行颜色。
- [Pane 标签栏组合](https://github.com/zed-industries/zed/blob/130f1e92a48ecdcc210419c5d9a52d605fa18fc0/crates/workspace/src/pane.rs#L3477)：标签与辅助操作在同一区域组合。

进一步 Git 克隆完整工作树后，还核对了 `ui/styles/spacing.rs` → `ui_macros/dynamic_spacing.rs` → Tab 尺寸、`editor::create_style` → gutter 字符度量 → `element` 的 wrap/content origin，以及 `markdown_preview` → `MarkdownStyle::with_preview_overrides`。Zed 的 UI 密度与 UI 字号共同决定间距，gutter 按字符及最少/已保留位数测量；最终 Markdown 预览段距 16px、代码 padding 12px、调用处关闭代码框。NxNote 借鉴缩放与组合方式，但保留用户指定固定 gutter、可配置行高、A 的 WYSIWYG；12px 段距和 28px 文档栏是本地紧凑策略，不声称与 Zed 全部尺寸一致。源码仅静态核查，未作视觉或运行验收。
