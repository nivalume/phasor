# barq 知识库

barq 源码的知识库：每页讲一件事（一个 crate、一个概念或一个算法），页面之间的链接、所属、依赖和相关关系组成一张图。

- **内容**在 [`content/`](content/index.md)：普通 Markdown，在 GitHub 上直接可读，从 [index.md](content/index.md) 开始。
- **网页**在 `src/`：React 应用，渲染同一批 Markdown，加上搜索、按页面类型分组的目录、每页的局部图谱和反向链接，以及整个知识库的关系图。

## 运行

需要 Node.js 18 或更新版本。

```bash
cd wiki
npm install
npm run dev          # 开发服务器，http://localhost:5173
npm run build        # 先跑 check 和类型检查，再输出静态站点到 dist/
npm run preview      # 本地预览 dist/
npm run check        # 检查页面：frontmatter、链接、锚点、源码路径、孤立页面
npm run check:mermaid  # 用 mermaid-cli 渲染每个 mermaid 块（见 skills/mermaid-diagrams）
npm test             # 内容模型的单元测试
```

`dist/` 是纯静态文件，路由用 `#/…`，放在任何静态托管或子路径下都能用。

## 页面

`content/<目录>/<id>.md`，文件名就是页面 id，全库唯一。目录决定页面类型：

| 目录 | `kind` | 写什么 | 图里的形状 |
|---|---|---|---|
| `guides/` | `guide` | 怎么读：全景、约定、路线、练习 | 黄色星形 |
| `crates/` | `crate` | 一个 crate 一页 | 粉色方块 |
| `concepts/` | `concept` | 跨函数的约定或数据结构 | 青色圆形 |
| `algorithms/` | `algorithm` | 一个核心算法一页 | 绿色菱形 |
| `reference/` | `reference` | 查表用的页面 | 紫色三角 |
| `tooling/` | `tooling` | 引擎之外的设施：研究协议、差分测试、基准 | 橙色十字 |

Frontmatter：

```yaml
---
title: 数量换算链与 percent        # 必填；正文第一行的 # 标题与它相同
kind: algorithm                    # 必填；与目录一致
summary: 一句话，160 字以内。        # 必填；用于搜索结果、悬停提示和图谱卡片
parent: barq-exec                  # concept / algorithm 必填：所属的 crate 或 tooling 页面
depends: [barq-core, barq-model]   # 只用于 crate 页面：Cargo.toml 里的依赖
related: [buy-leg, sell-leg]       # 可选：参见
tags: [execution, sizing]          # 可选
sources:                           # 可选：这页讲的代码，路径必须存在，可带 :行 或 :起-止
  - crates/barq-exec/src/kernel.rs:610-677
order: 802                         # 可选：同类页面里的顺序
---
```

crate 页面的小节顺序固定：定位 → 文件与阅读顺序 → 类图 → 核心算法与概念 → 调用链 → 阅读陷阱 → 与参考实现的对应 → 延伸资料 → 自测。概念和算法页以 `## 位置`（代码位置）开头，以 `## 相关` 结尾。

## 链接

所有链接都写相对路径，这样 GitHub 和网页里都能点：

- 到另一页：`[数量换算链](../algorithms/size-cascade.md)`，可以带锚点 `#小节标题`（GitHub 的规则：小写，空格变 `-`，去掉标点，中文保留）。从 `index.md` 出发写 `crates/barq-exec.md`。
- 到仓库文件：`[docs/plan.md](../../../docs/plan.md)`；图片：`![…](../../../docs/img/zh/bar-loop.svg)`。
- 代码位置写成行内代码：`` `crates/barq-exec/src/kernel.rs:144` ``。网页会把它变成 GitHub 链接（文件存在时才链接）。crate 页面及其子页面里也可以写相对 crate 的路径（`` `src/lib.rs` ``）。

代码链接打开 [`src/config.ts`](src/config.ts) 里 `CODE_REF` 指定的提交，页面里的行号都按那个提交核对过。代码改动后重新核对行号，再更新 `CODE_REF`（或构建时设置 `VITE_CODE_REF`）。

## 图谱里的关系

| 关系 | 来源 | 画法 |
|---|---|---|
| 正文链接 | 正文里指向另一页的链接 | 细实线 |
| 所属 | `parent` | 粗实线 |
| 依赖 | crate 页面的 `depends` | 粗虚线 |
| 相关 | `related`（无方向） | 点线 |

全局图谱里，crate 按依赖深度从左到右排列，概念和算法围在所属 crate 旁边，指南在上、参考和工具链在下；只有所属和依赖决定位置，正文链接和相关关系只画线。

## 代码结构

- `src/core/wiki.js`：内容模型（解析 frontmatter、链接、标题锚点，建图）。网页和 `scripts/check-wiki.mjs` 共用。
- `src/components/`：Markdown 渲染、力导向图（d3-force）、窗口、目录、搜索面板等。
- `src/pages/`：首页、页面、图谱三个视图。
- `src/styles/`：设计变量（`tokens.css`）、界面（`app.css`）、正文排版（`prose.css`）。浅色和深色两套。
