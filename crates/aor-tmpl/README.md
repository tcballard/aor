# aor-tmpl

Mature alternative: **Askama and MiniJinja**. Owns an interpreted template parser with Rust-derived context schemas. It rejects unknown fields in every branch, escapes output and rejects unsupported grammar. Release Rust code generation is still pending.

Experimental. See [implementation status](../../docs/status.md) for limits and unmet release gates. Plain `cargo build` and `cargo test` work at the workspace root.

Current grammar: `{{ field.path }}`, `if`/`else`/`endif`, `for item in list`/`endfor`, comments, `upper`, `lower`, `trusted`. Root context derives `TemplateContext`. Objects, lists, bools, strings and integers are supported. Trusted output requires the private `TrustedHtml` type constructed by the named plain-text sanitiser.

Dynamic output is limited to HTML text nodes. Dynamic attributes, script/style/textarea/title contexts are rejected. Includes, inheritance, enum match and the release proc-macro compiler are not yet implemented. Parsing, schema checking and rendering are bounded; no arbitrary Rust or expression evaluation is exposed.

Fuzz CPU-hours and crash-free completion dates: `../../docs/evidence/fuzz.json`, bound to the current template source hash. The 200-hour gate is unmet.

Recorded qualifying CPU-hours at this checkpoint: template parser **0**. Local ASan smoke completed on 30 September 2026 with LeakSanitizer unavailable and is ineligible for the release gate.
