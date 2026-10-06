# 03: Dual MIT / Apache-2.0 license baseline

**What to build:** A distributable tree. A legal reviewer / packager finds top-level license artifacts declaring MIT / Apache-2.0 dual-license terms with a matching manifest license field, so packaging and audit tooling agree and the tree is no longer "unlicensed". No Gate / Policy / Session code touched.

**Blocked by:** None (can start immediately).

**Status:** ready-for-agent

- [ ] Top-level license artifacts present (MIT + Apache-2.0 texts)
- [ ] Package manifest license field set to match artifacts so `cargo package` validates
- [ ] README license footer no longer claims "unlicensed"; states dual-license terms
- [ ] No change to Principal / Gate / Policy / Approver / Run / Session behavior
