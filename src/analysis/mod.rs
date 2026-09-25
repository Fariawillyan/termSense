//! Deterministic analyzers for shell and system notations: Unix
//! permissions, cron schedules, exit codes and sed/awk programs. Like
//! `regex/` and `networking/`, they know nothing about the knowledge base or
//! the terminal; the assistant turns their results into documents.

pub mod awk;
pub mod cron;
pub mod exit_code;
pub mod permissions;
pub mod sed;
