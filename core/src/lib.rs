//! The Assistant's core: talks to FastFlowLM, runs the agent loop and tools,
//! and keeps memory, skills and chats. `assistantd` exposes it over D-Bus.

pub mod agent;
pub mod flm;
pub mod paths;
pub mod store;
pub mod tool;

#[cfg(test)]
mod fake_flm;
