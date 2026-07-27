<!-- lang: en (default) -->
> **Language / 语言:** **English** · [中文](./PROTOCOL.zh.md)

# GPU Worker Command Protocol

The async runtime never touches DRM/GBM/EGL objects directly. It talks to the
`rabbit-gpu` thread only through the channels documented below.

## Commands (`GpuWorkerCommand`)

| Command | Direction | Effect |
| --- | --- | --- |
| `RegisterScreen { screen_id, frames }` | async → worker | Attach a KMS capture receiver for one screen |
| `SetScreenFrameRate { screen_id, frame_rate }` | async → worker | Cap emission rate for that screen |
| `ReleaseScreen(screen_id)` | async → worker | Drop capture + composition state |
| `RegisterPipeline { id, screen_id, parameters, outputs }` | async → worker | Subscribe a processed-frame consumer |
| `ReleasePipeline(id)` | async → worker | Drop one pipeline subscription |
| `Shutdown` | async → worker | Stop the worker loop |

## Notifications (`GpuWorkerNotification`)

| Notification | Direction | Effect |
| --- | --- | --- |
| `ScreenFailed { screen_id, error }` | worker → async | Fail all pipelines for the screen |

## Modules

| Module | Responsibility |
| --- | --- |
| `mod` (loop) | Command dispatch, screen/pipeline registry, thread lifetime |
| `composition` | Multi-plane KMS composition into a single DMA-BUF |
| `output` | Output strategy selection (passthrough / NV12 / VAAPI) |

## Invariants

1. Only the worker thread may own `GpuContext` / `GpuDevice`.
2. Frame delivery uses latest-value channels; slow consumers drop intermediate frames.
3. Errors on a screen fail every pipeline registered for that screen.
