//! Domain kernel — platform-independent capabilities and session protocol.
//!
//! # Domain map
//!
//! | Package surface | Modules | Responsibility |
//! | --- | --- | --- |
//! | **Protocol** | [`protocol`], [`connection_request`], [`session_control`], [`transport`] | Wire identity, handshake, control messages, channels |
//! | **Session** | [`session`] | Role-gated session API, RTP reassembly |
//! | **Screen** | [`geometry`], [`screen_manager`], [`screen_capture`], [`screen_configuration`] | Topology, capture ports, stream negotiation types |
//! | **Media** | [`frame_pipeline`], [`screen_stream`], [`video_encoder`], [`video_decoder`], [`video_renderer`] | Frame processing and encode/decode/present ports |
//!
//! Kernel code must not depend on GUI, OS-specific crates, or `infra`.
//! Platform adapters implement kernel traits; `app` orchestrates them.

pub mod connection_request;
pub mod frame_pipeline;
pub mod geometry;
pub mod protocol;
pub mod screen_capture;
pub mod screen_configuration;
pub mod screen_manager;
pub mod screen_stream;
pub mod session;
pub mod session_control;
pub mod transport;
pub mod video_decoder;
pub mod video_encoder;
pub mod video_renderer;
