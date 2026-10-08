//! Claude-style settings scopes: managed > CLI > local > project > user.
//!
//! Permission rules (`allow`/`ask`/`deny`) merge across scopes (lists
//! concatenate); scalars take the highest scope that sets them. Deny from
//! any scope beats allow from any other — decided in `rules.rs`.
