<!-- lang: zh -->
> **语言 / Language:** [English](./ARCHITECTURE.md) · **中文**

# Rabbit 架构

Rabbit 是用 Rust 编写的点对点远程桌面系统。本文是代码结构、数据流与扩展方式的
**权威地图**。

若改动与本文冲突，请在同一次变更中更新文档。

---

## 1. 目标

| 目标 | 架构如何支撑 |
| --- | --- |
| 低延迟屏幕串流 | 不可靠视频信道、DMA-BUF / 硬件编码、latest-frame 管线 |
| 清晰的产品演进 | 类型级平台栈（`ApplicationStack`），而非散落的 `cfg` |
| 安全并发 | Compio 异步编排 + 带回收的工作线程 |
| 可评审的增长 | `kernel`（领域）/ `infra`（适配）/ `app`（编排）/ `ui`（展示） |
| 单进程双角色 | Host 与 Controller 会话共存于同一 GUI 消息环 |

当前非目标：多协议联邦、云中继拓扑、超出串流控制的输入重定向。

---

## 2. 分层系统（端口与适配器）

```
┌──────────────────────────────────────────────────────────────────┐
│  ui/  (Slint)                                                     │
│    intents ──► app::gui::view ──► RootMessage 总线                │
│    ◄── ViewState / 视频呈现回调                                   │
└─────────────────────────────┬────────────────────────────────────┘
                              │
┌─────────────────────────────▼────────────────────────────────────┐
│  app/                                                             │
│    gui::application   消息环、分组 UI/运行时状态                    │
│    services/ + runtime/  纯策略（无展示壳依赖）                     │
│    model              会话目录与 ID                                 │
│    platform           ApplicationStack 选择与组装                   │
└───────────────┬─────────────────────────────┬────────────────────┘
                │                             │
┌───────────────▼───────────────┐   ┌─────────▼────────────────────┐
│  kernel/  （领域端口）         │   │  infra/  （适配器）            │
│  protocol, session, screen,   │   │  QUIC/TCP, KMS/WGC, GST/MF,  │
│  media traits & wire types    │   │  Wayland/D3D 渲染             │
└───────────────────────────────┘   └──────────────────────────────┘
```

### 依赖规则

```
ui → app → kernel
app → infra → kernel
kernel → ∅   （无 app、无 infra、无 GUI）
```

违反即架构缺陷（见 `src/architecture.rs`）。

---

## 3. 领域地图（`kernel`）

| 面 | 模块 | 职责 |
| --- | --- | --- |
| **协议** | `protocol`, `connection_request`, `session_control`, `transport` | 版本身份、握手、控制标签、信道/投递 |
| **会话** | `session`（`rtp`, `role`） | 角色门禁收发、RTP 组装、会话消息 |
| **屏幕** | `geometry`, `screen_manager`, `screen_capture`, `screen_configuration` | 拓扑、采集端口、串流协商类型 |
| **媒体** | `frame_pipeline`, `screen_stream`, `video_encoder`, `video_decoder`, `video_renderer` | 采集与网络/显示之间的处理端口 |
| **策略辅助** | `capability`, `domain_error` | 准入检查与结构化失败分类 |

### 协议版本与握手

`kernel::protocol` 拥有：

- `PROTOCOL_MAJOR` / `PROTOCOL_MINOR` / `PROTOCOL_NAME`
- 控制信道 ID 与视频屏 ID 预算

`kernel::connection_request::ConnectionRequest` 携带：

- 协议 major/minor
- `PeerCapabilities { max_screens, encoder_profiles }`
- 请求方显示名

对等方 **必须** 共享 major。major 不同时 Host 回复
`ConnectionResponse::ProtocolMismatch` 并丢弃请求。minor 可加；未知编码
profile 标签被忽略。

开流前经 `kernel::capability`（及 `app::runtime::host_policy`）校验，使宣告预算
真正生效。

传输信道映射 **必须** 与协议常量对齐。

### 入口

| API | 角色 |
| --- | --- |
| `rabbit::run()` | GUI Host + Controller（Slint） |
| `rabbit::run_headless()` / `--headless` | 无界面 Host，自动接受 Controller，共享 services/编码路径 |

### 会话角色

| 角色 | 可否发视频 | 典型控制 |
| --- | --- | --- |
| **Host** | 是 | 屏列表、串流已配置；接收开流/关键帧请求 |
| **Controller** | 否 | 开流请求、关键帧请求；接收屏列表/视频 |

角色检查在 `session::role` 与会话 API 边界强制，而非仅在 GUI。

### RTP 策略（Controller）

实现于 `session::rtp`：

1. 按时间戳组装直到 marker。
2. 序号缺口丢帧并请求 IDR。
3. 在完整关键帧恢复前忽略依赖帧。

---

## 4. 应用编排（`app`）

### 4.1 运行时拓扑

```
main
 └─ gui::run::<Stack>()
     ├─ UI 线程: Slint
     └─ rabbit-app 线程: Compio Runtime
          └─ RootApplication 消息环
               ├─ 连接监听任务
               ├─ 每会话接收任务
               ├─ Host 串流任务（编码 + 发送）
               └─ 可选远端视频解码任务
```

Headless 使用同一栈组装与 host 策略，无 Slint。

### 4.2 `RootApplication` 状态分组

| 分组 | 拥有 |
| --- | --- |
| `LifecycleState` | 关闭 / 结束 |
| `WorkspaceState` | 分区、状态文案、串流设置错误 |
| `ListenerState` | 本地绑定身份、直连 UI、accept 环 |
| `RemoteStreamState` | Controller 串流 UI + 解码器 |
| `HostStreamState` | Host 侧 pending start/stop |
| `ApplicationModel` | 会话、远端屏、ID 分配、平台 `App`、本地能力 |

### 4.3 消息流

```
GuiIntent  ──┐
后台任务     ├──► RootMessage ──► update 分发
             ──┘                      │
                    ┌─────────────────┼──────────────────┐
                    ▼                 ▼                  ▼
              lifecycle         connection           session
              remote_video      host_stream
                    │
                    └──► runtime/* + services/*（策略）/ model / kernel
                    └──► ViewPublisher（视图变更时）
```

`gui/application/update/*` 中的 handler 应保持轻薄：路由、拉起 I/O、调用
runtime/services、更新状态组。

### 4.4 应用服务与运行时策略

| 模块 | 用途 |
| --- | --- |
| `services::host_stream` | 根据屏与请求规划 host 管线 |
| `services::session_catalog` | Host 会话/串流稳定排序 |
| `runtime::host_policy` | 能力 + 相位准入后规划 |
| `runtime::session_lifecycle` | Joining/Active/Draining 穷尽迁移 |

这些模块不得依赖展示壳。

### 4.5 平台栈

```text
ApplicationStack
├── App: 布局 + 采集 + 帧管线 manager
├── RemoteVideo: 解码器 + 帧类型
├── RemoteVideoViewStack: native / OpenGL 呈现绑定
└── ScreenStreamEncoder: Host 编码器
```

| 栈 | 成熟度 |
| --- | --- |
| `linux/niri-kms-gbm-gstreamer-wayland` | 主产品路径 |
| `windows-wgc-d3d11-mf` | 脚手架；落后于 Linux |

新后端 = 新 `ApplicationStack` + `infra` 适配器。勿在会话协议代码中分支。

---

## 5. 端到端媒体路径

### Host（出站）

```
ScreenLayoutManager
    → ScreenCaptureManager.acquire(screen)
    → FramePipelineManager.subscribe(screen, size, fps)
    → VideoEncoder::run (H.264 RTP 包)
    → SessionSend::send_video (Unreliable, Video(screen_id))
```

### Controller（入站）

```
SessionRecv (RTP 组装)
    → ReceivedVideoFrame
    → VideoDecoder::run
    → VideoView (Wayland DMA-BUF / OpenGL / Slint)
```

### 控制面（可靠）

屏列表、启停串流、关键帧请求、配置结果 — 见 `session_control` 线格式标签。

---

## 6. Infra 分包说明

Linux 视频编码（`infra/.../video_encoder/gstreamer/`）：

| 模块 | 角色 |
| --- | --- |
| `frame` | DMA-BUF → GStreamer buffer + caps |
| `rtp` | Sample → RTP 包字节 |
| `pipeline_util` | Bus、元素、低延迟编码器配置 |
| `va_surface` / `probe` | VA 分配与时延探针 |
| 根 `gstreamer.rs` | 编码器生命周期 + 集成测试 |

帧管线 worker：`worker/{mod,composition,output}` 与
[`PROTOCOL.zh.md`](src/infra/platform/linux/frame_pipeline/worker/PROTOCOL.zh.md)。

---

## 7. 如何扩展（操作手册）

### 新增控制消息

1. 在 `kernel::screen_configuration`（或对等模块）加领域类型。
2. 在 `session_control` 加线标签与编解码。
3. 在 `session::role` / `SessionSend` / `SessionRecv` 强制角色。
4. 在对应 `update/*` handler 处理。
5. 兼容升 `PROTOCOL_MINOR`，破坏性升 `PROTOCOL_MAJOR`。

### 新增平台后端

1. 在 `infra/platform/<os>/` 实现 kernel trait。
2. 定义新 `ApplicationStack`。
3. 在 `app/platform/<os>.rs` 选择。
4. 为硬件路径添加 focused 测试 / scripts。

### 新增纯策略决策

1. 优先 `app/services/*` 或 `app/runtime/*` 并带单测。
2. 从 handler 调用；不要把分支写进 Slint 绑定。

---

## 8. 测试策略

| 层 | 风格 |
| --- | --- |
| `kernel` | 同目录单测；会话用假 transport |
| `app/services` / `app/runtime` | 确定性纯测 |
| `app/gui/state` | UI 状态机单测 |
| 架构 | `src/architecture.rs` 分层守卫 |
| 硬件媒体 | `scripts/test-*` / ignored lib 测试（需 GPU） |

先跑 focused 测试（`cargo test --lib <module>`）。仅编译不能代替模块 focused
测试（见 [`CONTRIBUTING.zh.md`](CONTRIBUTING.zh.md)）。

---

## 9. 不变量（勿轻易破坏）

1. **信道 0 为控制**；视频为 `screen_id + 1`。
2. **ScreenId::MAX == 254**；`255` 不是视频屏。
3. **Host 永不接收远端视频**；Controller 永不发送视频。
4. 会话层视频 **仅不可靠投递**。
5. **Kernel 保持与 OS 无关**。
6. 进程媒体路径在启动时选定 **一个 ApplicationStack**。
7. Host 串流使用 **latest-frame / 合并关键帧请求** — 勿在异步运行时无界积压帧。
8. Host 开流前必须做 **能力与相位检查**。

---

## 10. 路线图状态

已完成基础：

- [x] 三层布局 + 类型级栈
- [x] GUI 消息环模块化 + 根状态分组
- [x] 应用服务：host 规划 / 会话目录
- [x] Session 包拆分（`rtp`, `role`）
- [x] 协议版本常量 + 信道单源
- [x] GStreamer frame/rtp/util 抽出
- [x] 握手嵌入 major/minor + 能力；major 不匹配 → `ProtocolMismatch`
- [x] `RunningSession` 相位：Joining → Active → Draining
- [x] GPU worker 包：`composition` / `output` / loop + `worker/PROTOCOL.md`
- [x] 连接时能力宣告
- [x] Headless host：`rabbit::run_headless()` / `rabbit --headless`
- [x] 领域错误、能力协商、架构守卫、共享 host 策略

### 结构支柱

| 支柱 | 实现 |
| --- | --- |
| 领域错误 | `kernel::domain_error::{DomainErrorKind, DomainError}` |
| 能力协商 | `kernel::capability` + `runtime::host_policy` 作用于 SetScreenStreams |
| 穷尽会话相位表 | `runtime::session_lifecycle`（3×2 事件，单测） |
| 共享 host 策略 | GUI + headless 调用同一 `evaluate_set_screen_streams` |
| 会话上的对端能力 | 握手写入 `RunningSession.peer_capabilities` |
| 架构守卫 | `src/architecture.rs` 禁止 kernel→app/infra、services/runtime→GUI |

后续产品增量（非结构阻塞）：

1. 握手 **响应** 向 Controller 回传 host 能力。
2. 会话 FSM 超时 / 重连事件。
3. Headless controller / 录到文件 sink。
4. 继续拆分超大文件（gstreamer 编码主体）。

---

## 11. 术语

| 术语 | 含义 |
| --- | --- |
| **Host** | 采集并编码本机屏幕的对等方 |
| **Controller** | 请求串流并呈现视频的对等方 |
| **Stack** | 平台 manager + 编解码 + 呈现的编译期捆绑 |
| **RootMessage** | 异步任务与 GUI 环之间的应用内事件总线 |
| **Service / runtime policy** | 内核端口之上的纯应用层决策，供所有壳复用 |
