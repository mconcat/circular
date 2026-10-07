//! Operating-system realization of the Shell contracts.
//!
//! The product's first supported realization is a current-user launchd domain
//! on Apple Silicon macOS.  This module deliberately exposes no system-domain
//! or privilege-escalation path.

pub mod launch_agent;
pub mod location;
