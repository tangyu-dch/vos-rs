//! IVR 拓扑执行引擎模块
//!
//! 本模块实现基于拓扑画布（nodes/edges）的 IVR 图遍历执行引擎，
//! 与现有的扁平 DTMF 表 (ivr_actions) 并行存在，向后兼容。
//!
//! 入口：[`engine::TopologyEngine::execute`]。
//!
//! 注：本模块当前为 stub 阶段，尚未接入主呼入流程，故暂允许 dead_code；
//! 待后续阶段与 `ivr.rs` 集成后移除该 allow。

#![allow(dead_code, unused_imports)]

pub mod engine;
pub mod executors;
pub mod types;
pub mod voice_engine;

pub use engine::TopologyEngine;
pub use types::{
    IvrExecutionContext, IvrNodeType, IvrTopology, NodeExecuteResult, Position, TopologyEdge,
    TopologyGraph, TopologyNode,
};
pub use voice_engine::{AsrConfig, AsrEngine, TtsConfig, TtsEngine, TtsResult, VoiceEngineManager};
