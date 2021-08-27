#![allow(dead_code)]
#![allow(renamed_and_removed_lints)]
#![cfg_attr(feature = "cargo-clippy", allow(unreadable_literal))]

// We depend on nix generating a library and a rs file
// from a text manifest (located in message_types/)
include!("../message_source/eventmsgs.rs");
