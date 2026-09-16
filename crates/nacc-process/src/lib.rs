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
//! Every spec is validated before anything is created (`ProcessSpec`'
//! `validate`): an empty program, a NUL byte inside an argument (which the
//! OS would silently truncate the command line at, so the audit record
//! would name a command that was never run), an invalid environment
//! variable name, or a missing working directory are all refused up front.
//! Child environments are inherited by default and can be made an explicit
//! allowlist ([`ProcessSpec::isolated_environment`]) -- an inherited
//! environment is a real secret-exfiltration path, since a user's shell
//! often carries provider API keys and cloud credentials.
//!
//! What is deliberately **not** here yet: ConPTY/PTY support. The master
//! plan's S9.1/S9.2 do not require a pseudo-console for either Claude Code
//! or Codex (both have documented non-interactive modes with structured
//! output, and those are the modes Phase 5 uses), so a pseudo-console would
//! be speculative platform code with no consumer -- and `CapabilitySnapshot`
//! already carries `interactive_pty` as a real, provider-reported fact, so
//! its absence is visible rather than hidden. A PTY lands when a provider
//! genuinely requires one, with its own tests.
//!
//! **Provenance note**: a parallel Phase 3 attempt on this branch (commit
//! `e87cc3b`, superseded by this implementation and still reachable in
//! history) took a PTY-first approach with `portable-pty`/`win32job`. Its
//! two genuinely transferable ideas were ported here -- explicit
//! environment allowlisting and spec validation -- while its PTY session
//! machinery was not, because it replaced this crate's whole public API
//! (which Phases 4+ are already wired into) and no current adapter needs a
//! pseudo-console. That commit remains the starting point if and when an
//! interactive-console provider is implemented.

pub mod containment;
pub mod supervisor;

pub use containment::{process_alive, terminate_process, ContainmentError, JobObject};
pub use supervisor::{
    CancelMode, CapturedOutput, DiscardLines, LineSink, ProcessError, ProcessExit, ProcessLine,
    ProcessSpec, ProcessStream, ProcessSupervisor, Result, SupervisedProcess, TracingLineSink,
};
