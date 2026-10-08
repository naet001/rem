//! Filesystem guard: canonicalize + re-check at tool open.
//!
//! Second enforcement layer under `classify` (which stays pure): resolves
//! symlinks and re-checks the canonical path so TOCTOU and
//! requested-inside/target-outside escapes fail closed.
