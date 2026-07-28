<!-- lang: zh -->
> **语言 / Language:** [English](./CONTRIBUTING.md) · **中文**

# 为 Rabbit 做贡献

感谢参与 Rabbit。项目重视小而可评审、可验证的改动。

## 开发原则

- 每次改动聚焦一个定义清晰的问题。
- 不要把无关重构或格式化混进功能变更。
- 平台实现放在 `infra`，业务能力与核心数据类型放在 `kernel`，工作流编排放在 `app`。
- 禁止抑制 Rust 的 `dead_code` lint；应删除未使用代码、在生产路径中实际使用，或放入真实的模块边界。
- 平台条件编译只能写在平台实现内；仅 `app/mod.rs` 与 `infra/mod.rs` 中直接选择 `mod platform` 路径的声明例外。通用模块必须通过统一的平台接口调用。
- 含子模块的模块必须使用 `name/mod.rs` 布局；禁止同时使用 `name.rs` 与同级 `name/` 目录。`app::platform` 和 `infra::platform` 特意直接映射到 `platform/linux/mod.rs` 或 `platform/windows/mod.rs`。

## 架构

系统架构、依赖规则、媒体路径、扩展手册与不变量见
[`ARCHITECTURE.zh.md`](ARCHITECTURE.zh.md)。修改会话协议、分层边界或平台栈前请先阅读。

### 分层摘要

| 层 | 路径 | 职责 |
| --- | --- | --- |
| `kernel` | `src/kernel/` | 能力 trait、会话/控制协议、核心值类型 |
| `infra` | `src/infra/` | QUIC/TCP、平台采集/编解码/渲染 |
| `app` | `src/app/` | 配置、服务、GUI 工作流、栈组装 |
| UI | `ui/` | 经 `app::gui::view` 绑定的 Slint 视图 |

除非改动专属于 Windows，媒体路径优先落在 Linux 栈。

## 工作流

1. 动手前写清假设、范围与验收标准。
2. 实现新模块前先建立下文所述的 focused 测试。
3. 实现可独立评审的最小改动。
4. 对受影响模块运行 focused 测试。
5. 检查 diff，去掉无关变更。
6. 获批后再开始下一个独立改动。

## 模块测试优先

在实现或集成新模块之前：

1. 添加最小可编译模块边界与确定性 no-op/空实现。
2. 在模块旁添加 focused 单测，导入边界并成功运行。
3. 记录只跑该测试目标的精确命令。
4. 增量替换空实现，始终保持测试可运行且通过。

每个改动该模块的提交前都必须成功跑过其 focused 测试；失败或无法运行则不要提交。模块接入应用后仍保留该测试。

私有实现模块的测试使用同目录单元测试，以便不扩大生产 API 也能测私有边界。仅在测试有意公开的边界或跨多模块行为时使用 `tests/` 集成测试。

## Rust 验证

每次提交前运行受影响模块的 focused 测试。仅编译不能代替该测试。

按风险与评审要求选择其他验证：

```shell
cargo fmt --check
cargo check
```

涉及 lint、并发、安全边界或公开接口时，酌情运行：

```shell
cargo test
cargo clippy --all-targets
```

Windows 专属改动使用 `cargo-xwin` 交叉检查 Windows 目标：

```shell
cargo xwin clippy --target x86_64-pc-windows-msvc
```

某步无法运行时，在交接中说明原因。

## Linux 远程输入权限

远程键盘和鼠标注入使用 `/dev/uinput`。请通过 udev 规则为运行 Rabbit 的用户授予读写权限，不要以 root 身份运行 Rabbit：

```shell
sudo groupadd -f uinput
sudo usermod -aG uinput "$USER"
echo 'KERNEL=="uinput", GROUP="uinput", MODE:="0660"' \
  | sudo tee /etc/udev/rules.d/99-rabbit-uinput.rules
sudo udevadm control --reload-rules
sudo udevadm trigger --name-match=uinput
```

修改用户组后需注销并重新登录。

鼠标移动默认使用绝对定位。测试可靠相对移动时，在 `config.toml` 中设置：

```toml
[input]
pointer_mode = "relative"
```

## 提交约定

提交主题遵循 Conventional Commits：

```text
<type>(<scope>): <summary>
```

`scope` 可选。每个提交应表示一个可解释、可评审、可独立回滚的改动。

### Type

| Type | 用途 |
| --- | --- |
| `feat` | 增加用户可见能力或系统能力 |
| `fix` | 修复缺陷 |
| `refactor` | 不改变外部行为的结构调整 |
| `perf` | 性能改进 |
| `test` | 增加或更新测试 |
| `docs` | 仅文档 |
| `build` | 构建系统或依赖维护 |
| `ci` | CI 配置 |
| `chore` | 其他维护 |
| `revert` | 回滚已有提交 |

依赖引入新系统能力用 `feat(deps)`；升级/降级/维护已有依赖用 `build(deps)`。

### Scope

优先用标识边界的小写 scope：

- `app`：应用生命周期与工作流编排
- `kernel`：能力接口与核心数据类型
- `infra`：平台或外部系统实现
- `deps`：依赖
- `config`：配置
- `logging`：日志
- `docs`：跨多文档的文档结构

无清晰 scope 时可省略。不要为填字段而发明含糊 scope。

### Summary

- 使用以小写字母开头的英语祈使短语，例如 `add compio runtime dependency`。
- 描述完成结果，而非步骤。
- 不以句号结尾。
- 主题尽量不超过 72 字符。

### Body 与 Footer

简单改动可仅有主题。动机、权衡或行为不明显时，空一行后写正文，聚焦**为什么**。

破坏性变更在 type 或 scope 后加 `!`，并在 footer 说明：

```text
feat(protocol)!: replace legacy handshake

BREAKING CHANGE: peers using the legacy handshake can no longer connect.
```

关联议题用 `Refs:` 或 `Closes:`。

### 示例

```text
feat(deps): add compio runtime dependency
feat(kernel): add screen capture subscription interface
fix(infra): preserve screen layout after refresh failure
docs: add contribution and commit guidelines
refactor(app): separate session creation from dependency assembly
```
