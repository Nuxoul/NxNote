# Markdown 源码映射收敛

## 参考依据

已直接阅读本地固定提交的 Rust 源码，而非只读 README：

- Velotype `a61c71955c6104fc5d647088b5957da6d59149ca`：`src/components/block/runtime/projection.rs` 的双向坐标与标记边界，`src/editor/source_mapping.rs` 的正文/前缀映射。
- Lockbook `f2dcdf392e3ffac706bf1839652e67674b8e00c2`：`libs/content/workspace/src/tab/markdown_editor/widget/block/leaf/paragraph.rs` 明确将解析节点前后空白放回源码布局；`input/mutation.rs` 将结构操作与源文本替换分开。

本轮没有直接复制完整编辑器，也不宣称 Typora 等价性。

## 本轮收敛的不变量

1. 原 Markdown 文本仍是唯一正文真值；掩码只用于解析。显示必须恢复原文，不能把掩码的 `x` 带入画面。
2. 普通字符和空白保留源码位置；Unicode 字素、解码实体/转义、图片与局部渲染卡片按明确完整范围处理。
3. 左右导航读取当前投影的 SourceMap，不依赖上一帧字形几何；上下导航和 Home/End 在几何失效时有物理行回退。
4. 输入、粘贴、选区替换、复制/剪切及删除使用一致范围校正。IME 已开始组合时保留原生预编辑范围，不强制扩大为显示原子。
5. 投影重建时使旧几何失效；窗口几何只负责鼠标命中、软换行位置、选区绘制与候选框。
6. 引用定义保留可编辑原文；链接目的地从真实源文本解析树收集，不从掩码后的解析树收集。
7. 未能精确映射的解码片段回退原文，不制造零长度字符映射。实体识别由显示和编辑共用。

## 涉及模块

- 新增 `src/source_map.rs`，由 Document 的显示单位生成停止点、字素范围和原子范围，并提供导航/替换范围 Interface。
- `src/note_editor.rs` 接入当前 SourceMap；移除仅保护图片的独立输入规则；统一复制和剪切范围，表格内删除也使用当前投影单位。
- `src/document_layout.rs` 移除 `source_steps`，合并同源码范围显示标签的命中片段；同位置别名的左右命中具有明确选择规则。
- `src/markdown_projection.rs` 恢复掩码文本、保留原链接定义、共享实体解码，并继续补齐物理行首尾空白。
- `src/markdown_edit.rs` 共用实体解码；高亮/上下标在 Enter 拆段时复用普通强调的成对标记规则。

## 检查与限制

已添加当前投影导航、空行回退、引用原子删除、组合字素、普通符号空格、掩码不泄漏、引用定义、IME 标点、格式拆段和命中的回归用例，**未执行**。

6 个涉及 Rust 文件使用 `rustfmt --edition 2021 --config skip_children=true --emit stdout` 语法解析成功；输出只在会话 scratch。Git 空白差异检查通过，新模块也单独检查过。

**没有编译、类型检查、运行测试、启动程序或桌面自动化。** 本轮不能证明所有 Markdown 编辑问题已解决；当前变更尚未打包或推送。图片标签、用户笔记、配置和旧发布文件不改动。
