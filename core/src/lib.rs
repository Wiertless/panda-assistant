//! The Assistant's core: talks to FastFlowLM, runs the agent loop and tools,
//! and keeps memory, skills and chats. `assistantd` exposes it over D-Bus.

pub mod agent;
pub mod context;
pub mod flm;
pub mod mcp;
pub mod paths;
pub mod service;
pub mod store;
pub mod tool;
pub mod tools;

#[cfg(test)]
mod fake_flm;
