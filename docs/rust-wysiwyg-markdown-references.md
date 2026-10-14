# Rust 所见即所得 Markdown 编辑器源码调研

本轮任务是寻找真正的 Rust 渲染式 Markdown 编辑器参考，不是修复实现。已搜索 GitHub、浅克隆并阅读两个项目的 Rust 源码；没有编译、测试、启动应用，也没有修改 NxNote 的编辑器代码。因此以下为源码核实结果，不是交互验收或 Typora 等价性声明。

## 1. 主参考：Velotype（Rust + GPUI）

- 仓库：https://github.com/manyougz/velotype
- 固定提交：`a61c71955c6104fc5d647088b5957da6d59149ca`
- [Cargo.toml](https://github.com/manyougz/velotype/blob/a61c71955c6104fc5d647088b5957da6d59149ca/Cargo.toml#L1-L71)：版本 0.7.5、Rust 2024、GPUI 0.2、pulldown-cmark。与 NxNote 的 Kit / GPUI-pre 版本并不相同，不能直接视为可插拔控件。
- [README](https://github.com/manyougz/velotype/blob/a61c71955c6104fc5d647088b5957da6d59149ca/README.md#L23-L32) 明确提供原生渲染式编辑和源码模式；不是 WebView 编辑器。项目自称仍处于早期阶段，IME 仍列于路线图。
- [原生输入](https://github.com/manyougz/velotype/blob/a61c71955c6104fc5d647088b5957da6d59149ca/src/components/block/input.rs)：Rust `impl EntityInputHandler for Block`。WYSIWYG 核心确实在 Rust 中，并非仅有 Rust 桌面壳。

### 已读的编辑规则

1. [BlockKind](https://github.com/manyougz/velotype/blob/a61c71955c6104fc5d647088b5957da6d59149ca/src/components/block/state.rs#L111-L156) 独立表示段落、标题、列表、引用、表格、代码和原文回退。格式不是仅靠隐藏字符串标点表达。
2. [`on_newline`](https://github.com/manyougz/velotype/blob/a61c71955c6104fc5d647088b5957da6d59149ca/src/components/block/interactions.rs#L300-L447)：空列表请求 `RequestOutdent`；普通渲染段落拆分内容并发出 `RequestNewline`；代码/表格等有单独语义。
3. [`newline_sibling_kind`](https://github.com/manyougz/velotype/blob/a61c71955c6104fc5d647088b5957da6d59149ca/src/components/block/state.rs#L300-L314)：列表创建同类项、任务项重置未勾选，标题后创建普通段落。
4. [`on_delete_back`](https://github.com/manyougz/velotype/blob/a61c71955c6104fc5d647088b5957da6d59149ca/src/components/block/interactions.rs#L450-L543)：行首先按类型处理——标题转普通段落，列表退缩进/降级；之后才处理空块删除和与上一块合并。不能把“解除格式”和“删除正文”混为一个邻近可见字符删除。
5. [编辑器级事件](https://github.com/manyougz/velotype/blob/a61c71955c6104fc5d647088b5957da6d59149ca/src/editor/events.rs#L1714-L1759) 负责插入新块、焦点和撤销事务。跨块选区另由 `src/editor/selection.rs` 管理；不是一个 Textarea 在块间临时切换。
6. [ExpandedInlineProjection](https://github.com/manyougz/velotype/blob/a61c71955c6104fc5d647088b5957da6d59149ca/src/components/block/runtime/projection.rs#L181-L196) 明确保存 clean→display、display→clean、展开符号区间，并在后续方法处理光标 affinity、左右移动和命中。正文边界与展开标记边界并非一个无偏置 offset。
7. [源码前缀双向映射](https://github.com/manyougz/velotype/blob/a61c71955c6104fc5d647088b5957da6d59149ca/src/editor/source_mapping.rs#L16-L70) 将空根段落、列表/引用前缀与正文坐标分开。
8. [空行导入](https://github.com/manyougz/velotype/blob/a61c71955c6104fc5d647088b5957da6d59149ca/src/editor/document.rs#L988-L1047) 显式生成空段落；一般块间首个空行用于 Markdown 分隔，额外空行生成空段落，列表间另有处理。**这不是逐字保留原始 Markdown 的保证。**

### 不直接照搬

- 它是块树运行时、规范化 Markdown 序列化，且有源码模式/不稳定语法回退。NxNote 仍需保留连续文档体验，不能要求用户切模式或点击段落进入编辑。
- 不能直接采用规范化保存并重写现有笔记的空白、列表符号或链接写法。参考结构化操作和坐标不变量，并生成最小源文本编辑事务更符合当前数据兼容要求。
- 图片显示不替代 NxNote 的文件名标签、悬浮预览和引用删除约定。
- [LICENSE-APACHE](https://github.com/manyougz/velotype/blob/a61c71955c6104fc5d647088b5957da6d59149ca/LICENSE-APACHE)：Apache-2.0。若实际复制/改编代码，应遵守许可、版权与修改标识要求；本轮未复制实现。

## 2. 单文档源文本参考：Lockbook（Rust + egui）

- 仓库：https://github.com/lockbook/lockbook
- 固定提交：`f2dcdf392e3ffac706bf1839652e67674b8e00c2`
- 当前源码位置为 `libs/content/workspace/src/tab/markdown_editor`，不是旧版 `libs/editor`。
- [工作区依赖](https://github.com/lockbook/lockbook/blob/f2dcdf392e3ffac706bf1839652e67674b8e00c2/libs/content/workspace/Cargo.toml#L1-L55)：Rust、comrak、egui、glyphon。不是 GPUI；仅作为 Rust 渲染式 Markdown 编辑算法参考，不迁回 egui。
- [文档渲染](https://github.com/lockbook/lockbook/blob/f2dcdf392e3ffac706bf1839652e67674b8e00c2/libs/content/workspace/src/tab/markdown_editor/widget/block/container/document.rs#L29-L76) 在非 plaintext 模式绘制 Markdown 子节点；空文档仍建立零长度布局锚点，供光标和命中使用。
- [标题渲染](https://github.com/lockbook/lockbook/blob/f2dcdf392e3ffac706bf1839652e67674b8e00c2/libs/content/workspace/src/tab/markdown_editor/widget/block/leaf/heading.rs#L124-L176) 直接对源文本范围生成 Layout；语法按 reveal 显隐，空标题回退到整行语法显示。因此可称为原生渲染式/语法按需显示的编辑，不等同于完全不显示标记的富文本编辑器。

### 已读的关键规则

1. [`calc_operations` / Newline](https://github.com/lockbook/lockbook/blob/f2dcdf392e3ffac706bf1839652e67674b8e00c2/libs/content/workspace/src/tab/markdown_editor/input/mutation.rs#L58-L193)：事件转为 Buffer Operation；依据最深容器、物理行、`line_content`、`line_own_prefix` 判断续项或终止。空容器只删自己的前缀，不删后续换行；普通空行走默认插入换行。
2. [Delete 分支](https://github.com/lockbook/lockbook/blob/f2dcdf392e3ffac706bf1839652e67674b8e00c2/libs/content/workspace/src/tab/markdown_editor/input/mutation.rs#L194-L265)：先处理折叠原子区域/图片，再在内容开头处理容器自己的前缀，否则删除明确 Region。不要在全篇寻找最近可见字形作为删除目标。
3. [空行间隔范围](https://github.com/lockbook/lockbook/blob/f2dcdf392e3ffac706bf1839652e67674b8e00c2/libs/content/workspace/src/tab/markdown_editor/widget/block/spacing.rs#L9-L94)：用 parent、previous sibling 与 source_lines 定义 pre/post spacing，涵盖容器内部而不止根级节点。
4. 坐标使用显式 `Grapheme` / `Bytes` 类型，编辑事件与渲染布局分开。UI 类型不能直接导入 NxNote，但可借鉴操作边界的职责分离。
5. 图片有先选中再删除的规则，与 NxNote 的文件名标签 × 行为不同，不能机械复制。
- [UNLICENSE](https://github.com/lockbook/lockbook/blob/f2dcdf392e3ffac706bf1839652e67674b8e00c2/UNLICENSE)：仓库根许可为 Unlicense。依赖与独立资源的许可仍须分别核对；本轮未复制实现。

## 候选筛选与结论

- **首选同栈参考：Velotype。** 实际拥有 Rust + GPUI 渲染式 Markdown 编辑器，不再用 Zed 代替 WYSIWYG 参考。
- **源文本事务交叉参考：Lockbook。** 同为 Rust 原生渲染编辑，但不是 GPUI。更贴近 NxNote 保留源文本作为真值的要求。
- [Ferrite](https://github.com/OlaProeis/Ferrite) README 描述 rendered edit session / one-click block switching，不作为本次连续文档交互的主参考；未对其实现做完整核查。
- [ethemdemirkaya/markdown-editor](https://github.com/ethemdemirkaya/markdown-editor) README 明确为 Tauri + Svelte + TypeScript + Milkdown，Rust 主要是后端，不能作为 Rust 编辑核心证据。
- Zed 继续仅作 GPUI 原生输入/IME 契约参考，不是 WYSIWYG Markdown 项目。

## 实现方向与本轮落地

保留 GPUI Kit 界面、单一文档输入/选区/撤销与现有 Markdown 文件；将换行、格式降级、容器前缀删除、字符删除、段落合并改为独立的语义操作，再统一输出源文本事务及选区映射。投影只负责显示和双向命中，不决定“该删哪段文字”。物理空行与 Markdown 结构间距分别建模。继续保留用户要求的图片标签及真实路径复制，不引入源码/预览切换。

### 本轮源码实现

- 新增 `src/markdown_edit.rs`：借鉴 Velotype 的结构操作分流和 Lockbook 的物理行/当前层前缀处理，统一产生源文本及选区事务，不复制外部编辑器实现。
- Enter 在空容器上只解除一层前缀，其他情况下续列表/引用或拆段；有序列表同时支持 `.` / `)`，任务项重置未勾选；行内强调和链接拆段时平衡两侧标记。
- Backspace 在内容开头先处理标题降级/列表退层；字符、解码字符、图片原子和相邻行合并独立处理。Delete 在标题末尾不再清除标题格式。
- `NoteEditor` 将上述事务以一次撤销记录接入，保持单文档选区、IME 和图片原子引用；移除旧 `continuation_edit` 和 `DocumentGeometry::deletion_range`，绘制几何不再决定正文删除目标。
- 普通段落按物理源码换行生成显示行，保留独立空行锚点，段内换行不额外增加段落间距；多行语法原子保持完整映射。引用、代码围栏、分隔线和标题后缀可在编辑边界显示。
- 没有规范化重写整篇笔记，没有引入源码/预览切换，没有改变图片文件名标签、悬浮预览、× 删除引用与真实路径复制规则。

新增连续回车、空列表退出、逐层降级、正文符号、格式拆段、Unicode/图片原子、实体和物理行投影的回归用例。已用 `rustfmt --emit stdout` 解析涉及源码（输出仅在会话临时目录），并检查 Git 空白差异；**未编译、未做类型检查、未运行用例、未启动程序或交互验收**。一次可选只读复查因未及时返回已取消，不计为通过。本轮改动尚未打包到 EXE。
