# 01: Multi-profile config with env-only keys

**What to build:** A developer can declare multiple named providers in one configuration with a single active-provider pointer, each entry carrying its provider type, key reference, optional base URL, and default model. Secrets are referenced by environment variable only, and the old single-profile shape is rejected with a migration hint.

**Blocked by:** None (can start immediately).

**Status:** ready-for-agent

- [ ] Declares an active-provider pointer plus a map of named provider entries, each with provider type (OpenAI-compatible vs Anthropic native), key reference, optional base URL, and default model name
- [ ] Resolves keys via `env:VAR` indirection only; fails closed when the prefix is missing, the variable name is empty, or the variable is unset, naming the provider and variable
- [ ] Defaults the Anthropic base URL to its public endpoint when unset; OpenAI-compatible entries require an explicit base URL
- [ ] Rejects the legacy single-profile shape at load with an error showing the new multi-profile shape
- [ ] Keeps the theme pointer behavior unchanged and the default effort at medium when unset
- [ ] Covered by parsing tests: valid multi-provider, missing env failure, literal-key rejection, legacy-shape rejection with hint
