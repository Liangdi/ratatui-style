# ratatui-style

[![crates.io](https://img.shields.io/crates/v/ratatui-style.svg)](https://crates.io/crates/ratatui-style)
[![docs.rs](https://docs.rs/ratatui-style/badge.svg)](https://docs.rs/ratatui-style)
[![MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**[English](README.en.md)**

一个面向 [ratatui](https://ratatui.rs) 的 CSS 级联引擎 —— 选择器、优先级、继承、伪状态、数据驱动样式。
输出原生 ratatui `Style` / `Block` / `Constraint`，不做并行渲染。

使用标准 CSS 属性名（`color`、`background-color`、`font-weight`、`border`、`padding`、`margin`、`text-align`、`width` …），
支持 `serde`（服务端驱动的 UI 可通过 JSON 传输样式），实现级联规则：
**来源层 × 优先级 × 继承 × 伪状态**。

## 快速开始

```rust
use ratatui_style::{CssStyle, Origin, OwnedNode, Stylesheet};

let mut sheet = Stylesheet::new();
sheet.add(
    "Button.primary",
    CssStyle::new().color("#fff").background("blue").bold(),
    Origin::User,
)?;

let node = OwnedNode::new("Button").with_classes(["primary"]);
let computed = sheet.compute(&node, None);

// 投影到原生 ratatui 类型：
let _style   = computed.to_style();         // → ratatui::style::Style
let _block   = computed.to_block();         // → ratatui::widgets::Block
let _area    = computed.apply_margin(area); // 缩小 Rect
let _layout  = computed.constraints();      // → (Constraint, Constraint)
let _align   = computed.alignment();        // → Alignment
```

## CSS 文本样式表

```rust
use ratatui_style::Stylesheet;

let sheet = Stylesheet::parse(r#"
    :root { --accent: #00d4ff; }

    Button.primary {
        color: var(--accent);
        background: blue;
        font-weight: bold;
        border: rounded;
        padding: 0 2;
    }
    Button:focus { background: green; }
    #save:disabled { color: gray; }
"#)?;
```

## 级联模型

级联按五个步骤为每个元素解析样式：

1. **收集** 所有选择器匹配该节点的规则。
2. **排序** 按 `(来源层, 优先级, 源码顺序)` 升序。
3. **叠加** 声明 —— 后出现的规则逐字段覆盖前面的。
4. **继承** —— 可继承属性（`color`、`font-weight`、`font-style`、`text-decoration`、`underline-color`、`text-align`）从父元素的计算样式流入子元素的 `None` 字段。
5. **解析** `var()` 引用，对照令牌表替换为具体值。

### 来源层

规则按来源分层；同等优先级下，高来源层覆盖低来源层：

| 来源 | 优先级 | 用途 |
|---|---|---|
| `UserAgent` | 最低 | 内置默认值 |
| `Theme` | | 应用级主题 |
| `User` | | 用户配置 / CSS 文本 |
| `Inline` | 最高 | 行内样式 |

### 优先级（Specificity）

`(id数, class数+伪类数, 类型数)` —— 标准 CSS 优先级，以元组形式比较。
`*`（通用选择器）为 `(0, 0, 0)`。

## 支持的 CSS 属性

| 属性 | 值类型 | 映射到 |
|---|---|---|
| `color` | [颜色](#颜色语法) | `Style::fg` |
| `background` / `background-color` | 颜色 | `Style::bg` / `Block::style` |
| `font-weight` | `bold` / `normal` / `100`–`900` | `Modifier::BOLD` |
| `font-style` | `italic` / `normal` | `Modifier::ITALIC` |
| `text-decoration` | `underline` / `line-through` / 两者组合 | `Modifier::UNDERLINED` / `CROSSED_OUT` |
| `underline-color` | 颜色 | `Style::underline_color` |
| `border` | `none` / `single` / `rounded` / `double` / `thick` [颜色] | `Block::borders` + `border_type` |
| `padding` | `1` / `1 2` / `1 2 3` / `1 2 3 4` | `Block::padding` |
| `margin` | 同 padding 简写 | `Rect` 缩小 |
| `text-align` | `left` / `center` / `right` | `Alignment` |
| `width` / `height` | `auto` / `10` / `50%` / `min(3)` / `max(5)` | `Constraint` |

## 颜色语法

所有颜色属性支持：

| 语法 | 示例 |
|---|---|
| 十六进制 3/4/6/8 位 | `#fff` `#fff0` `#ff8800` `#ff8800ff` |
| `rgb()` / `rgba()` | `rgb(255, 128, 0)` `rgba(0,0,0,0.5)` |
| CSS 命名颜色 | `red` `blue` `cyan` `orange` `gold` … |
| `transparent` / `none` / `reset` | 重置为终端默认 |
| `inherit` | 强制从父元素继承 |
| `var(--name)` | CSS 自定义属性，可带回退值：`var(--accent, #fff)` |

## 选择器与伪类

复合选择器格式 `Type.class#id:pseudo…`，支持逗号列表和 `*` 通配：

```
Button                /* 类型 */
.primary              /* 类 */
#save                 /* id */
Button.primary:focus  /* 复合 */
Text, .muted, #title  /* 逗号列表 */
*                     /* 通配 */
```

伪类：`:focus` `:hover` `:disabled` `:checked` `:active`

## 继承与 `var()`

`color`、`font-weight`、`font-style`、`text-decoration`、`underline-color`、`text-align`
可从父元素的计算样式继承。`var(--name)` 从 `:root` 令牌表解析
（也可通过编程方式构建 `ThemeTokens`，或从 themekit 桥接）。

```rust
use ratatui_style::{CssStyle, Origin, OwnedNode, Stylesheet};

let mut sheet = Stylesheet::new();
sheet.tokens_mut().insert("accent", "#00d4ff");

sheet.add("Panel", CssStyle::new().color("#cdd6f4").italic(), Origin::Theme)?;
sheet.add("Button", CssStyle::new().background("var(--accent)").bold(), Origin::User)?;
sheet.add("Button:disabled", CssStyle::new().color("gray"), Origin::User)?;

// Panel 解析自身样式
let panel = sheet.compute(&OwnedNode::new("Panel"), None);

// Text 从 Panel 继承 color + italic
let text = sheet.compute(&OwnedNode::new("Text"), Some(&panel));

// 禁用按钮：:disabled 规则生效，color=gray
let btn = sheet.compute(
    &OwnedNode::new("Button").with_state(ratatui_style::State::disabled()),
    Some(&panel),
);
```

## 框架集成

在你的节点类型上实现 `StyledNode` —— 引擎不依赖任何特定框架：

```rust
use ratatui_style::{StyledNode, State, Position};

impl StyledNode for MyNode {
    fn type_name(&self) -> &str { &self.kind }
    fn id(&self) -> Option<&str> { self.id.as_deref() }
    fn classes(&self) -> Vec<&str> { self.classes.iter().map(String::as_str).collect() }
    fn state(&self) -> State { self.state }
    fn position(&self) -> Position { self.position.clone() }
}
```

## Feature 标志

| Feature | 默认 | 说明 |
|---|---|---|
| `serde` | ✅ | 为所有值类型提供 `Serialize`/`Deserialize` —— JSON 属性映射、配置文件、传输格式 |
| `themekit` | ❌ | `ThemeTokens::from_themekit` —— 将 `ratatui-themekit` 语义颜色槽桥接为 CSS `var()` 令牌 |

禁用默认 feature 可获得零依赖的纯样式引擎：

```toml
[dependencies]
ratatui-style = { version = "0.1", default-features = false }
```

## 示例

```sh
# 交互式仪表盘 —— 纯 CSS 驱动，单一样式表
cargo run --example dashboard

# 级联演示 —— 继承、var()、优先级、伪状态
cargo run --example cascade

# CSS 文本样式表解析
cargo run --example stylesheet

# 颜色与值解析
cargo run --example values

# themekit 桥接（需要 themekit feature）
cargo run --example themekit_bridge
```

## 生态定位

| Crate | 定位 | `ratatui-style` |
|---|---|---|
| `ratatui-themekit` | 15 个语义颜色槽 + 调色板 | **组合使用** —— `ThemeTokens::from_themekit` 填充 CSS 变量 |
| `tui-theme-builder` | 编译期 `Style` 宏 | `ratatui-style` 覆盖 **运行时/配置驱动** 场景 |
| `lipgloss` | "终端 CSS"（自有渲染栈） | 同类 DX，基于 ratatui 的 buffer 模型 |

## 当前状态

已实现：CSS 文本解析器、复合选择器、优先级、级联层（`UserAgent` < `Theme` < `User` < `Inline`）、
伪状态、`var()`（含回退值）、继承、盒模型（`padding` / `margin` / `border`）、
尺寸（`width` / `height` → `Constraint`）、`serde` 集成、`themekit` 桥接。

计划中：后代/子代组合器（`A B`、`A > B`）、`:nth-child`、`@media`、`ComputedStyle` 缓存。

## 许可证

MIT
