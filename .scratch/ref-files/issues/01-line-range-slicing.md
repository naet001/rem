# 01: Line-range slicing for file mentions

**What to build:** A coder can point at lines with `@file:START-END`, `@file:START-`, and `@file:LINE` and the model sees only that slice with stable line numbers. Invalid ranges block submit with a notice naming the bad ref. Existing `@file`, `@dir/`, multi-ref ordening, confinement, caps, and block semantics stay unchanged.

**Blocked by:** None (can start immediately).

**Status:** ready-for-agent

- [ ] Closed range inlines only those 1-indexed lines with line numbers and a header naming the ref as written
- [ ] Open range (`START-`) inlines from START to end-of-file
- [ ] Single line (`LINE`) inlines just that line with its number
- [ ] `END` past end-of-file clamps without error; `START` past end-of-file blocks naming the ref
- [ ] `START < 1`, `END < START`, non-numeric range block submit naming the ref (never warn-and-skip)
- [ ] Range on a directory blocks with a type-error notice naming the ref
- [ ] Sliced output still respects the per-file char cap with a truncation note
- [ ] Multi-ref mixing plain and ranged refs preserves appearance order in the model payload
- [ ] Transcript user row, input history, and title fallback keep the short original; sessions store the expanded payload
- [ ] Emails stay literal; trailing sentence punctuation trims; absolute and escaping paths stay blocked; `@` reads never pop the approval modal
- [ ] Covered at the submit-time expansion seam with temp-dir fixtures: single, closed, open, clamped END, each invalid form, range-on-dir, cap on slice, mixed multi-ref order
