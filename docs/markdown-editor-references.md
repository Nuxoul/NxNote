# Markdown 编辑器源码参考

本次按用户要求实际执行 `git ls-remote` 和浅克隆，并阅读以下第一方源码。仓库克隆位于会话临时目录 `markdown-references/`，不作为 NxNote 的依赖，不复制外部实现到项目。

| 项目 | 固定提交 | 克隆目录 |
| --- | --- | --- |
| [MarkText / Muya](https://github.com/marktext/marktext) | `b7e0124e163070828c90d547f7c9487278d2cc51` | `markdown-references/marktext`（稀疏检出 `packages/muya`） |
| [ProseMirror commands](https://github.com/ProseMirror/prosemirror-commands) | `52a84a842774fadec3b167bcdbd56085ec6c85df` | `markdown-references/prosemirror-commands` |

## 核对并采用的规则

### 删除字符与清除格式分离

Muya 的 [`_scanBackspaceTokens`](https://github.com/marktext/marktext/blob/b7e0124e163070828c90d547f7c9487278d2cc51/packages/muya/src/block/base/format.ts#L1503-L1547) 在标记内部按字素删除，调用方按删除长度恢复光标；清除格式由独立的 [`clearFormat`](https://github.com/marktext/marktext/blob/b7e0124e163070828c90d547f7c9487278d2cc51/packages/muya/src/block/base/format.ts#L155-L229) 操作完成。

NxNote 修正：不再在标记内部任意位置按一次 Backspace 就移除两端全部标记。标记内部只删除一个字素；`Ctrl+\` 清除格式。保留 NxNote 的明确边界行为：格式内容起点 Backspace / 终点 Delete 可以退出该层格式、保留文字，作为与 Muya 不完全相同的业务规则。

ProseMirror 的 [`atBlockStart` / `joinBackward`](https://github.com/ProseMirror/prosemirror-commands/blob/52a84a842774fadec3b167bcdbd56085ec6c85df/src/commands.ts#L15-L85) 把块结构操作限定在块起点，且 [`joinTextblocksAround`](https://github.com/ProseMirror/prosemirror-commands/blob/52a84a842774fadec3b167bcdbd56085ec6c85df/src/commands.ts#L97-L122) 校验事务确实缩短文档。NxNote 保留选区删除优先，普通内容删除、标记编辑与去格式是不同分支，空修改不会创建格式编辑。

### 标记边缘必须能看到语法

Muya [`getClassName` / `_checkConflicted`](https://github.com/marktext/marktext/blob/b7e0124e163070828c90d547f7c9487278d2cc51/packages/muya/src/inlineRenderer/renderer/index.ts#L117-L143) 使用 [`conflict`](https://github.com/marktext/marktext/blob/b7e0124e163070828c90d547f7c9487278d2cc51/packages/muya/src/utils/index.ts#L90-L92) 的闭区间判断。光标在标记两端时标记也应显示，不只在严格内部显示。

NxNote 修正：强调、链接、行内代码与标题前缀使用包含边界的显示规则。鼠标拖选期间仍固定当前投影，结束后更新，避免选区在拖动过程中因标记显隐漂移。

### 光标移动只采用真实内容边界

Muya [`getCursor` / `setCursor`](https://github.com/marktext/marktext/blob/b7e0124e163070828c90d547f7c9487278d2cc51/packages/muya/src/block/base/content.ts#L649-L676) 保留原文偏移和块身份；[`offsetCursor`](https://github.com/marktext/marktext/blob/b7e0124e163070828c90d547f7c9487278d2cc51/packages/muya/src/selection/offsetCursor.ts#L1-L75) 在跨引擎转换时重新解析光标，不以不同表示的下标简单相加。

NxNote 保留一份 Markdown 正文，显示字符记录原文字节范围。修正 `source_steps()`：隐藏结构的边界别名只用于定位，不作为普通左右键的额外停点；空行仍保留可编辑位置，图片维持原子引用边界。

### 输入法与撤销边界

Muya [`composeHandler`](https://github.com/marktext/marktext/blob/b7e0124e163070828c90d547f7c9487278d2cc51/packages/muya/src/block/base/content.ts#L683-L694) 和 [键盘处理](https://github.com/marktext/marktext/blob/b7e0124e163070828c90d547f7c9487278d2cc51/packages/muya/src/block/base/content.ts#L913-L946) 对组合输入单独处理；[历史分类](https://github.com/marktext/marktext/blob/b7e0124e163070828c90d547f7c9487278d2cc51/packages/muya/src/history/index.ts#L81-L105) 在输入类型改变、插入空白等边界分组。

NxNote 不直接复制 DOM/ZWSP 技巧：GPUI `EntityInputHandler` 必须即时返回预编辑文本和候选框布局，不能简单跳过绘制或恢复失效选区。继续保留输入法提交前不对外保存预编辑正文、提交后形成撤销事务。采用空白字符切断连续插入的撤销分组；删除、去格式、图片引用操作保持独立事务。

## 验证范围

只执行仓库克隆、源码阅读和 Git 差异检查。没有编译、测试、启动 NxNote 或桌面自动化。参考源码说明采用规则的依据，不证明 NxNote 已达到完整 Typora 交互一致性。
