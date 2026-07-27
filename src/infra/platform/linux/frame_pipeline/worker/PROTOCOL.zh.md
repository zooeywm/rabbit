<!-- lang: zh -->
> **语言 / Language:** [English](./PROTOCOL.md) · **中文**

# GPU Worker 命令协议

异步运行时从不直接触碰 DRM/GBM/EGL 对象。它仅通过下文所述信道与
`rabbit-gpu` 线程通信。

## 命令（`GpuWorkerCommand`）

| 命令 | 方向 | 效果 |
| --- | --- | --- |
| `RegisterScreen { screen_id, frames }` | async → worker | 为某屏挂接 KMS 采集接收端 |
| `SetScreenFrameRate { screen_id, frame_rate }` | async → worker | 限制该屏输出帧率 |
| `ReleaseScreen(screen_id)` | async → worker | 丢弃采集与合成状态 |
| `RegisterPipeline { id, screen_id, parameters, outputs }` | async → worker | 订阅处理后的帧消费者 |
| `ReleasePipeline(id)` | async → worker | 丢弃一条管线订阅 |
| `Shutdown` | async → worker | 停止 worker 循环 |

## 通知（`GpuWorkerNotification`）

| 通知 | 方向 | 效果 |
| --- | --- | --- |
| `ScreenFailed { screen_id, error }` | worker → async | 使该屏所有管线失败 |

## 模块

| 模块 | 职责 |
| --- | --- |
| `mod`（循环） | 命令分发、屏/管线注册表、线程生命周期 |
| `composition` | 多平面 KMS 合成为单一 DMA-BUF |
| `output` | 输出策略选择（直通 / NV12 / VAAPI） |

## 不变量

1. 仅 worker 线程可拥有 `GpuContext` / `GpuDevice`。
2. 帧投递使用最新值信道；慢消费者丢弃中间帧。
3. 某屏出错会使注册在该屏上的每条管线失败。
