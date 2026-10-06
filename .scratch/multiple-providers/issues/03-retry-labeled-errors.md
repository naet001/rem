# 03: Fixed retry with labeled turn errors

**What to build:** Transient provider failures during a Run (and during context-compaction summarization) are retried a fixed number of times with backoff, and the final failure surfaces as a turn error labeled with the active provider and model so the user knows which leg failed.

**Blocked by:** 02: Provider factory with live switch preserving history.

**Status:** ready-for-agent

- [ ] Retries 429/5xx/timeout up to 3 attempts with exponential backoff on the main chat call
- [ ] Applies the same retry wrapper to the compaction summarizer call so it is not the flaky path
- [ ] Passes non-retryable errors through immediately without retry
- [ ] Returns the final failure as a turn error labeled with the active `provider:model`; history is not dropped on failure
- [ ] Covered by stub-based tests with injected transient errors asserting attempt count and final labeled error; no live network calls
