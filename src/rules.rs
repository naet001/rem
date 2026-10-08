//! Allow/ask/deny rule engine: `Tool` / `Tool(specifier)` syntax.
//!
//! Evaluation order is fixed: deny → ask → allow. Specificity never
//! reorders. Bare-name deny removes the tool from the model's context.
