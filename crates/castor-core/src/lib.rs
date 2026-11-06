//! Castor core.
//!
//! Everything a tool needs, and nothing about the user interface:
//! - [`tool`]: the [`Tool`] trait and its static description ([`ToolMeta`]).
//! - [`options`]: the declarative option schema. The app builds every tool screen from it,
//!   the CLI parses `--set key=value` with it, chains store it.
//! - [`files`]: [`FileInfo`] and file-kind detection.
//! - [`job`]: [`Job`], [`Output`] and the [`RunCx`] handed to `Tool::run`.
//! - [`runner`]: runs a batch or a chain on worker threads and streams [`runner::RunEvent`]s.
//! - [`history`], [`chain`], [`settings`], [`paths`], [`download`], [`process`], [`i18n`].

pub mod chain;
pub mod download;
pub mod files;
pub mod history;
pub mod i18n;
pub mod job;
pub mod options;
pub mod paths;
pub mod process;
pub mod registry;
pub mod runner;
pub mod settings;
pub mod tool;
pub mod units;

pub use files::{FileInfo, FileKind};
pub use i18n::{tr, tr_args};
pub use job::{CancelToken, Job, Output, RunCx};
pub use options::{Choice, Cond, OptionKind, OptionSpec, Value, Values};
pub use registry::Registry;
pub use tool::{Family, InputSpec, Preview, Requirement, Tool, ToolMeta, ToolStatus};

/// Result type used across Castor.
pub type Result<T> = anyhow::Result<T>;

rust_i18n::i18n!("../../locales", fallback = "en");
