//! Networking as a first-class category: entity extraction, subnet math and
//! reference tables. Independent from the UI and from search.

pub mod analyzer;
pub mod knowledge;

pub use analyzer::{Cidr, NetQuery, ipv4_net, parse_cidr, scan};
