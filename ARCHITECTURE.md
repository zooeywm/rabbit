<!-- lang: en (default) -->
> **Language / 语言:** **English** · [中文](./ARCHITECTURE.zh.md)

# Rabbit Architecture

Rabbit is a peer-to-peer remote-desktop system written in Rust. This document is
the **canonical map** of how the codebase is structured, how data moves, and how
to extend it without eroding boundaries.

If a change conflicts with this document, update the document in the same change.

---

## 1. Goals

| Goal | How the architecture supports it |
| --- | --- |
| Low-latency screen streaming | Unreliable video channels, DMA-BUF / hardware encode paths, latest-frame pipelines |
| Clear product evolution | Type-level platform stacks (`ApplicationStack`), not scattered `cfg` |
| Safe concurrency | Compio async orchestration + owned worker threads with reaping |
| Reviewable growth | `kernel` (domain) / `infra` (adapters) / `app` (orchestration) / `ui` (presentation) |
| Dual role in one process | Host and Controller sessions coexist under one GUI message loop |

Non-goals today: multi-protocol federation, cloud relay topology, input remoting
beyond stream control.

---

## 2. Layered system (ports & adapters)

```
┌──────────────────────────────────────────────────────────────────┐
│  ui/  (Slint)                                                     │
│    intents ──► app::gui::view ──► RootMessage bus                 │
│    ◄── ViewState / video present callbacks                        │
└─────────────────────────────┬────────────────────────────────────┘
                              │
┌─────────────────────────────▼────────────────────────────────────┐
│  app/                                                             │
│    gui::application   message loop, grouped UI/runtime state      │
│    services/ + runtime/  pure policies (no presentation shell)  │
│    model              session catalog & IDs                       │
│    platform           ApplicationStack selection & assembly       │
└───────────────┬─────────────────────────────┬────────────────────┘
                │                             │
┌───────────────▼───────────────┐   ┌─────────▼────────────────────┐
│  kernel/  (domain ports)      │   │  infra/  (adapters)          │
│  protocol, session, screen,   │   │  QUIC/TCP, KMS/WGC, GST/MF,  │
│  media traits & wire types    │   │  Wayland/D3D renderers       │
└───────────────────────────────┘   └──────────────────────────────┘
```

### Dependency rule

```
ui → app → kernel
app → infra → kernel
kernel → ∅   (no app, no infra, no GUI)
```

Violations of this rule are architecture bugs (see `src/architecture.rs`).

---

## 3. Domain map (`kernel`)

| Surface | Modules | Responsibility |
| --- | --- | --- |
| **Protocol** | `protocol`, `connection_request`, `session_control` (`wire`), `transport` | Version identity, handshake, control tags/codecs, channels/delivery |
| **Session** | `session` (`rtp`, `role`) | Role-gated send/recv, RTP assembly, session messages |
| **Screen** | `geometry`, `screen_manager`, `screen_capture`, `screen_configuration` | Topology, capture port, stream negotiation types |
| **Media** | `frame_pipeline`, `screen_stream`, `video_encoder`, `video_decoder`, `video_renderer` | Processing ports between capture and network/display |
| **Policy helpers** | `capability`, `domain_error` | Admission checks and structured failure kinds |

### Protocol version & handshake

`kernel::protocol` owns:

- `PROTOCOL_MAJOR` / `PROTOCOL_MINOR` / `PROTOCOL_NAME`
- control channel id and video screen-id budget

`kernel::connection_request::ConnectionRequest` carries:

- protocol major/minor
- `PeerCapabilities { max_screens, encoder_profiles }`
- requester display name

Peers **must** share a major version. Hosts reply with
`ConnectionResponse::ProtocolMismatch` and drop the request when majors differ.
On **accept**, the host returns `ConnectionHandshakeReply::Accepted { host_capabilities }`
so the controller stores peer budgets on its session. Minor is additive; unknown
encoder profile tags are ignored.

Stream setup consults `kernel::capability` (and `app::runtime::host_policy` /
`host_control`) so advertised budgets are enforced.

Transport channel mapping **must** stay aligned with protocol constants.

### Entry points

| API | Role |
| --- | --- |
| `rabbit::run()` | GUI Host + Controller (Slint) |
| `rabbit::run_headless()` / `rabbit headless` | Headless Host, auto-accept controllers, shared services/encode path |
| `rabbit::run_record()` / `rabbit record` | Local screen → MP4 (path from config; screen/duration from CLI) |

### Session roles

| Role | May send video | Typical control |
| --- | --- | --- |
| **Host** | Yes | Screen list, stream configured, receive stream requests / keyframe requests |
| **Controller** | No | Stream requests, keyframe requests, receive screen list / video |

Role checks live in `session::role` and are enforced at the session API boundary,
not only in the GUI.

### RTP policy (Controller)

Implemented in `session::rtp`:

1. Reassemble packets by timestamp until the marker bit.
2. Drop frames on sequence gaps; request an IDR.
3. Ignore dependent frames until a complete keyframe restores the stream.

---

## 4. Application orchestration (`app`)

### 4.1 Runtime topology

```
main
 └─ gui::run::<Stack>()
     ├─ UI thread: Slint
     └─ rabbit-app thread: Compio Runtime
          └─ RootApplication message loop
               ├─ connection listener task
               ├─ per-session receive tasks
               ├─ host screen-stream tasks (encode + send)
               └─ optional remote video decoder task
```

Headless uses the same stack assembly and host policies without Slint.

### 4.2 State groups on `RootApplication`

| Group | Owns |
| --- | --- |
| `LifecycleState` | shutdown / finished |
| `WorkspaceState` | section, status copy, stream settings error |
| `ListenerState` | local bind identity, direct-connect UI, accept loop |
| `RemoteStreamState` | controller stream UI state + decoder |
| `HostStreamState` | pending host start/stop acknowledgements |
| `ApplicationModel` | sessions, remote screens, ID allocators, platform `App`, local capabilities |

### 4.3 Message flow

```
GuiIntent  ──┐
Background   ├──► RootMessage ──► update dispatcher
tasks        ──┘                      │
                    ┌─────────────────┼──────────────────┐
                    ▼                 ▼                  ▼
              lifecycle         connection           session
              remote_video      host_stream
                    │
                    └──► runtime/* + services/* (policy) / model / kernel
                    └──► ViewPublisher (if view changed)
```

Handlers in `gui/application/update/*` should stay thin: route, spawn I/O,
call runtime/services, update state groups.

### 4.4 Application services & runtime policies

| Module | Purpose |
| --- | --- |
| `services::host_stream` | Plan host pipelines from screens + request |
| `services::session_catalog` | Stable ordering of host sessions/streams |
| `runtime::host_policy` / `host_control` | Host admission + control classification |
| `runtime::controller_policy` | Controller stream request admission |
| `runtime::host_stream_launch` | Shared encode-task spawn for GUI/headless |
| `runtime::host_stream_lifecycle` | Stream finish/stop bookkeeping (GUI + headless) |
| `runtime::session_lifecycle` | Phase table, timeouts, reconnect eligibility |

These modules must not import the presentation shell.

### 4.5 Platform stacks

```text
ApplicationStack
├── App: layout + capture + frame pipeline managers
├── RemoteVideo: decoder + frame type
├── RemoteVideoViewStack: native / OpenGL present binding
└── ScreenStreamEncoder: host encoder
```

| Stack | Maturity |
| --- | --- |
| `linux/niri-kms-gbm-gstreamer-wayland` | Primary product path |
| `windows-wgc-d3d11-mf` | Scaffolded; trails Linux |

New backends = new `ApplicationStack` impl + `infra` adapters. Do not branch
inside session protocol code.

---

## 5. End-to-end media paths

### Host (outbound)

```
ScreenLayoutManager
    → ScreenCaptureManager.acquire(screen)
    → FramePipelineManager.subscribe(screen, size, fps)
    → VideoEncoder::run (H.264 RTP packets)
    → SessionSend::send_video (Unreliable, Video(screen_id))
```

### Controller (inbound)

```
SessionRecv (RTP assemble)
    → ReceivedVideoFrame
    → VideoDecoder::run
    → VideoView (Wayland DMA-BUF / OpenGL / Slint)
```

### Control plane (reliable)

Screen list, set/stop streams, keyframe request, configuration outcomes — see
`session_control` wire tags.

---

## 6. Infra packaging notes

Linux video encoder (`infra/.../video_encoder/gstreamer/`):

| Module | Role |
| --- | --- |
| `frame` | DMA-BUF → GStreamer buffer + caps |
| `rtp` | Sample → RTP packet bytes |
| `discovery` | VAAPI VPP input format/modifier discovery |
| `encoder` | Long-lived H.264 encode pipeline |
| `pipeline_util` | Bus, elements, low-latency encoder config |
| `va_surface` / `probe` | VA allocator & latency probes |
| root `gstreamer.rs` | Thin re-exports (`hardware_h264_encoder_for`) |
| `tests` | Focused GStreamer integration tests |

Frame pipeline worker: `worker/{mod,composition,output}` plus
[`PROTOCOL.md`](src/infra/platform/linux/frame_pipeline/worker/PROTOCOL.md).

---

## 7. How to extend (playbooks)

### Add a control message

1. Add domain type in `kernel::screen_configuration` (or peer module).
2. Add wire tag + codec in `session_control`.
3. Enforce role in `session::role` / `SessionSend` / `SessionRecv`.
4. Handle in the appropriate `update/*` handler.
5. Bump `PROTOCOL_MINOR` (compatible) or `PROTOCOL_MAJOR` (breaking).

### Add a platform backend

1. Implement kernel traits under `infra/platform/<os>/`.
2. Define a new `ApplicationStack`.
3. Select it from `app/platform/<os>.rs`.
4. Add focused tests / scripts for the hardware path.

### Add a pure policy decision

1. Prefer `app/services/*` or `app/runtime/*` with unit tests.
2. Call it from handlers; do not embed branching in Slint bindings.

---

## 8. Testing strategy

| Layer | Style |
| --- | --- |
| `kernel` | Co-located unit tests; fake transports for session |
| `app/services` / `app/runtime` | Deterministic pure tests |
| `app/gui/state` | UI state machine unit tests |
| Architecture | `src/architecture.rs` layering guards |
| Hardware media | `scripts/test-*` / ignored lib tests (GPU required) |

Run focused tests first (`cargo test --lib <module>`). A compilation-only check is
not a substitute for the module's focused test (see
[`CONTRIBUTING.md`](CONTRIBUTING.md)).

---

## 9. Invariants (do not break casually)

1. **Channel 0 is control**; video is `screen_id + 1`.
2. **ScreenId::MAX == 254**; `255` is not a video screen.
3. **Host never accepts remote video**; Controller never emits video.
4. **Unreliable delivery only for video** at the session layer.
5. **Kernel stays OS-free**.
6. **One ApplicationStack** selected at process start for the process media path.
7. **Latest-frame / coalesced keyframe commands** on host streams — do not queue unbounded frames on the async runtime.
8. **Capability and phase checks** before host stream admission.

---

## 10. Roadmap status

Completed foundation:

- [x] Three-layer layout + type-level stacks
- [x] GUI message loop modularization + grouped root state
- [x] Application services for host planning / session catalog
- [x] Session package split (`rtp`, `role`)
- [x] Protocol version constants + channel single source of truth
- [x] GStreamer frame/rtp/util extraction
- [x] Handshake embeds `PROTOCOL_MAJOR/MINOR` + capabilities; major mismatch → `ProtocolMismatch`
- [x] `RunningSession` phases: Joining → Active → Draining
- [x] GPU worker package: `composition` / `output` / loop + `worker/PROTOCOL.md`
- [x] Capability advertisement (`max_screens`, encoder profile tags) on connect
- [x] Headless host: `rabbit::run_headless()` / `rabbit --headless`
- [x] Domain errors, capability negotiation, architecture guards, shared host policy

### Structural pillars

| Pillar | Implementation |
| --- | --- |
| Domain errors | `kernel::domain_error::{DomainErrorKind, DomainError}` |
| Capability negotiation | `kernel::capability` + `runtime::host_policy` on SetScreenStreams |
| Exhaustive session phase table | `runtime::session_lifecycle` (3×5 events + timeouts, unit-tested) |
| Shared host policy | GUI + headless call the same `evaluate_set_screen_streams` / `classify_host_session_message` |
| Peer caps on sessions | `RunningSession.peer_capabilities` from handshake |
| Session timeouts / reconnect | `SessionTimeoutPolicy`, `evaluate_reconnect`; shells supersede draining peers |
| Host stream lifecycle | `host_stream_lifecycle` finish/stop helpers shared by shells |
| Architecture guards | `src/architecture.rs` forbids kernel→app/infra, services/runtime→GUI |

Further product increments (not structural blockers):

1. Headless controller / record-to-file sinks.
2. Continue splitting remaining mega-files (frame pipeline worker, gstreamer encode body).
3. Surface peer capabilities in the Slint UI.

---

## 11. Glossary

| Term | Meaning |
| --- | --- |
| **Host** | Peer that captures and encodes local screens |
| **Controller** | Peer that requests streams and presents video |
| **Stack** | Compile-time bundle of platform managers + codecs + presenters |
| **RootMessage** | Internal app event bus between async tasks and the GUI loop |
| **Service / runtime policy** | Pure app-layer decisions above kernel ports, reused by all shells |
