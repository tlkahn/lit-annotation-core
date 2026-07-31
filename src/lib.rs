//! Shared scanner, parser, scope resolver, and emitter for the Lit annotation DSL.
//!
//! Consumed by the Lit desktop app and the Lif mobile reader so both run the
//! identical grammar. Grammar changes ship as new tags; consumers pin tags and
//! never track `main`.

pub mod types;
pub mod lang;
pub mod scanner;
pub mod compact;
pub mod block;
pub mod marks;
pub mod parser;
pub mod emit;
pub mod scope_resolver;
#[cfg(test)]
mod round_trip;
