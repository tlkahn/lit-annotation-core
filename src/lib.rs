//! Shared scanner, parser, scope resolver, and emitter for the Lit annotation DSL.
//!
//! Consumed by the Lit desktop app and the Lif mobile reader so both run the
//! identical grammar. Grammar changes ship as new tags; consumers pin tags and
//! never track `main`.

pub mod block;
pub mod compact;
pub mod emit;
pub mod lang;
pub mod marks;
pub mod parser;
#[cfg(test)]
mod round_trip;
pub mod scanner;
pub mod scope_resolver;
pub mod types;
