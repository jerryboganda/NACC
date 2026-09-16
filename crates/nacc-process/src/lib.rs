//! Windows Job Object process containment, line-framed output streaming,
//! and graceful/forced cancellation (master plan S10, S13.4, S16).
//!
//! Phase 3 scope, second half. The first half (`nacc-git`, `nacc-runtime`)
//! landed as pure subprocess wrapping with no exotic Win32 surface; this
//! half is the platform-API half the phased plan deliberately separated
//! out -- and it is verified by tests that spawn real, multi-level process
//! trees and then ask the OS whether the descendants are really gone,
//! rather than trusting that a cancel call returned `Ok`.
//!
//! What is here:
//!
//! - [`containment`] -- `JobObject` (`KILL_ON_JOB_CLOSE`), plus
//!   [`containment::process_alive`] for crash-recovery reconciliation and
//!   [`containment::terminate_process`] as the documented direct-child
//!   fallback.
//! - [`supervisor`] -- [`ProcessSupervisor::spawn`] /
//!   [`SupervisedProcess::cancel`] / [`SupervisedProcess::wait`], with
//!   [`LineSink`] delivery of every output line.
//!
//! What is deliberately **not** here yet: ConPTY/PTY support. The master
//! plan's S9.1/S9.2 do not require a pseudo-console for either Claude Code
//! or Codex (both have documented non-interactive modes with structured
//! output, and those are the modes Phase 5 uses), so a pseudo-console would
//! be speculative platform code with no consumer -- and `CapabilitySnapshot`
//! already carries `interactive_pty` as a real, provider-reported fact, so
//! its absence is visible rather than hidden. A PTY lands when a provider
//! genuinely requires one, with its own tests.

pub mod containment;
pub mod supervisor;

pub use containment::{process_alive, terminate_process, ContainmentError, JobObject};
pub use supervisor::{
    CancelMode, DiscardLines, LineSink, ProcessError, ProcessExit, ProcessLine, ProcessSpec,
    ProcessStream, ProcessSupervisor, Result, SupervisedProcess, TracingLineSink,
};
