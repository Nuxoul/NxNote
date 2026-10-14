# Rust + GPUI 编辑器参考与本轮修正

## 实际克隆的同栈项目

- 项目：[zed-industries/zed](https://github.com/zed-industries/zed)
- 固定提交：`26363aabdb31a0c016728634202e1ad5067033aa`
- 执行方式：`git ls-remote` 后浅克隆、blobless 稀疏检出 `crates/editor/src`、`crates/gpui/src`、`crates/markdown/src`、`crates/text/src`。
- 会话目录：`rust-gpui-references/zed`。外部仓库不进入项目依赖，不直接复制其编辑器代码。

**技术栈已确认**：Zed 的 `crates/editor/src/input.rs` 为 Rust，并实现其 GPUI `EntityInputHandler`。**Zed 不是 Typora 式所见即所得 Markdown 编辑器**；`crates/markdown` 提供解析和展示，不是现成可替换的 Markdown 输入控件。它的 multi-buffer、rope、display-map 和当前 GPUI 版本与 NxNote 的 `gpui-kit 0.7.3 / gpui-pre 0.3.9` 不能直接替换。

## 采用的同栈契约

### 原生输入接口必须实现完整

[GPUI `EntityInputHandler`](https://github.com/zed-industries/zed/blob/26363aabdb31a0c016728634202e1ad5067033aa/crates/gpui/src/input.rs#L13-L123) 的默认 `paste` 只插入文本，默认 `set_selected_text_range` 无操作，默认 `text_length_utf16` 返回 `None`。当前安装版本的对应 `gpui-pre` 接口也已直接阅读核对。

NxNote 已补齐原生图片/文字粘贴、原生 UTF-16 选区更新和 UTF-16 正文长度。原生粘贴与 `Ctrl+V` 使用同一业务入口，图片仍保存为外部文件引用及文件名标签。

### UTF-16 范围采用偏置裁剪

Zed 的 [预编辑范围和选区处理](https://github.com/zed-industries/zed/blob/26363aabdb31a0c016728634202e1ad5067033aa/crates/editor/src/input.rs#L2960-L3070) 对起点使用 `Bias::Left`，终点使用 `Bias::Right`，并用事务包住组合输入。

NxNote 的 UTF-16→UTF-8 转换现按相同边界不变量处理：起点不能跳到 surrogate pair 后面，非空范围终点向右覆盖完整字符，空选区保持为空。预编辑相对选区使用同样裁剪，再根据表格转义后的实际 UTF-8 字节长度映射。

### 预编辑和提交不能走两套文本规范化

Zed [组合输入事务](https://github.com/zed-industries/zed/blob/26363aabdb31a0c016728634202e1ad5067033aa/crates/editor/src/input.rs#L2960-L3047) 与普通输入均进入编辑器输入处理器。当前 Kit 的 Rust 输入状态 `gpui-base/src/input/base/state.rs:4384–4503` 会即时更新预编辑文字、显示映射和选区；未提供相对选区时默认落在预编辑末尾。**不能从浏览器 DOM 的实现推断 GPUI 应保留旧选区或完全停止重排。**

NxNote 保留 GPUI 的即时预编辑显示和末尾默认光标，修正以下差异：

- 原生普通输入和预编辑均校正图片原子引用范围。
- 表格内预编辑和正式提交均转义竖线、换行，选区按规范化后的文本重新计算。
- 解析副本暂时保护预编辑里的 Markdown 标点；绘制仍取真实原文及对应坐标，提交后恢复正常解析。
- 空预编辑结束组合事务，必要时同步正文变更；确认前对外保存仍使用最后已提交正文。

### 原生命中和拖选定位必须区分

Zed 的 [原生 `character_index_for_point`](https://github.com/zed-industries/zed/blob/26363aabdb31a0c016728634202e1ad5067033aa/crates/editor/src/input.rs#L3122-L3137) 首先检查 `text_hitbox`，再按实际位置映射和偏置返回 UTF-16 位置。

NxNote 原生命中现先限制在编辑视口内；拖动选区仍允许越出视口并使用最近文字位置自动滚动。候选框位置使用同一显示布局的当前视觉行范围，不将下一行的 x 坐标拼接到第一行。

### 几何生命周期核对

Zed 用每帧 PositionMap 保存原文/显示映射。本轮也核对了当前 GPUI `canvas(prepaint, paint)` 的实际定义：NxNote 清空几何的回调是**第一个 prepaint 回调**，发生在行元素重新登记几何之前，不是 paint 回调。因此“绘制前清空本帧所有坐标导致光标消失”的调研推断不成立，没有据此修改生命周期。

## 检查范围与限制

已执行克隆、固定提交核对、Rust 源码阅读及 `git diff --check`。没有编译、测试、启动应用或桌面自动化。以上是同栈接口和实现不变量的修正依据，不表示 NxNote 已通过交互验收，也不表示找到现成的 GPUI Typora 克隆。
