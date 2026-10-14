# Markdown 扩展实现状态

## 当前结论

本轮为未完成的实现，不是可发布版本。保留此前用户的工作区修改、笔记与发布 EXE；没有编译、类型检查、测试、程序启动或桌面自动化。不能将已写入模块等同于整体功能已可用。

## 已写入并做语法解析检查的模块

- `src/markdown_extensions.rs`：高亮、上下标、行内/块公式、Mermaid 围栏、Wiki 双链/嵌入、脚注、GitHub 提示块、Front Matter 的 UTF-8 源码范围扫描。包含未执行的回归用例。
- `src/markdown_renderers.rs`：RaTeX → SVG → resvg PNG 原生公式渲染；Mermaid 原生 SVG/PNG 渲染；syntect 多语言代码高亮。后台调用、缓存文件、输入和图形尺寸限制、禁用 SVG 外部资源加载。未实际生成图像验证。
- `src/note_links.rs`：当前文件夹优先的笔记解析、重名错误、路径限制、HTTP/HTTPS/mailto 校验、标题章节提取、只读摘录、自嵌入拒绝和不递归展开。
- `src/markdown_extension_projection.rs`：扩展显示叠加与代码着色逻辑，**尚未从投影主文件声明为子模块**。
- `src/markdown_widgets.rs`、`src/markdown_widget_layout.rs`：后台资产状态、元数据卡片、公式图表卡片、只读嵌入摘录、导航与脚注返回接口。
- `src/note_editor.rs`、`src/document_layout.rs`、`src/app.rs`：已开始接入后台资产、样式、卡片和应用层导航。尚未做类型检查，不保证全部接口与借用通过编译。
- `src/markdown_format.rs`、`src/markdown_edit.rs`：增加扩展标记解除和块公式/元数据的原文换行规则。
- Cargo.toml / Cargo.lock：已解析并固定渲染依赖；`cargo metadata --locked --no-deps` 及 Windows x64 的 `cargo fetch --locked` 由独立模块实现代理执行成功，无构建。

## 必须继续完成的接入

1. `src/markdown_projection.rs` 编辑因同一用户轮次三次失败触发编辑安全停止，具体工具错误为 `EDIT_LINES_UNSEEN`。本轮不再编辑该文件，也不通过 shell 或其他代理绕过限制。
2. 下一轮先读取实时源码后，声明 `#[path = "markdown_extension_projection.rs"] mod extension_projection;`；补齐 `collect_links`；为所有 `Inline` 穷尽匹配加入 `Widget`（尤其 `push_text_lines` / `inline_start` / `inline_end`）；接入代码块语言信息与 `color_code_rows`。当前主文件已经调用未接入的 `apply_extensions`，因此当前工作区不能作为可构建完成版本交付。
3. 块扩展掩码和行内扩展转换须进一步核对：脚注定义可能被 GFM 当引用定义隐藏；提示块头部必须替换成明确标题，不能只叠加颜色；折叠块内部空行不能由空行补齐逻辑重复生成；行内样式与普通强调混合时要保持范围与内容完整。
4. 笔记重命名没有稳定 ID / 历史别名：必须实现安全的链接维护或别名方案，并与 Session::rename 的失败保存保护集成。当前 `nxnote:` 只解决文件夹歧义，不解决重命名。
5. 嵌入目前显示有限只读原文摘录，不是完整的递归 Markdown 嵌入渲染；有意不展开内部嵌入以防循环。
6. 后台结果失效、缓存上限、重复请求、错误提示、脚注返回坐标以及 GPUI 借用/回调需做具体收尾核查。投影当前未接通，尚未验证这些交互。
7. 完成接入后追加集成回归用例。遵守用户限制不执行，除非用户随后授权；打包请求仅授权 Release 构建、依赖检查与哈希检查，不授权运行测试或启动程序。

## 支持范围限制

- 不提供任意 HTML/CSS/JavaScript 渲染，不新增 WebView。
- 公式为 RaTeX 支持的 TeX 子集，不支持任意宏和外部资源。
- Mermaid 使用 Rust renderer 支持的流程图、时序、类、状态、ER、饼图、思维导图子集，不宣称浏览器 Mermaid 全兼容。
- 代码高亮不是编译器语义分析；未知/超限语言回退纯文本。
- 当前图片文件名标签、悬浮预览、× 仅删除引用的约定不改变。

## 本轮实际检查

12 个涉及的 Rust 文件使用 `rustfmt --edition 2021 --config skip_children=true --emit stdout` 解析成功，输出只写会话 scratch。`git -c core.safecrlf=false diff --check` 返回 0。这是语法与空白检查，**不是编译、类型检查、用例通过或交互验收**。现有 EXE 不包含本轮未完成的扩展实现。

## 后续打包更新（2026-10-14）

以上“无法构建/尚未接入”描述为上一实现轮次状态。本次打包已接入扩展投影子模块、链接收集、Widget 范围匹配、代码高亮；修正折叠块的空行补齐、脚注定义显示和提示块标题。首次 Release 编译发现 StyledExt 导入及注释布局借用冲突，已修复。

`cargo build --locked --release` 成功（重试用时 2m09s），未运行测试或启动程序。已检查 PE 为 x64 Windows GUI、依赖仅 Windows 系统 DLL；发布复制与构建产物 SHA-256 一致。

- 交付文件：`dist/NxNote-v0.2.1-md-20261014-1041-windows-x64.exe`
- 大小：27,811,328 字节。
- SHA-256：`66B51C3620CE69B97FEC107FBA103DBCA921437FA05D871F199AE18793A8D6EF`。

这份 EXE 可供用户手动验收，不代表新增交互已通过。仍未实现重命名后的双链维护；嵌入仍为有限只读原文摘录，而非完整 Markdown 嵌入渲染。公式/Mermaid 仍受上述子集限制。旧发布文件未覆盖。
