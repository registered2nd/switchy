# Learnings

Transferable heuristics captured from past sessions on this project. These are rules of thumb for future work — not history (that's `SESSION_LOG.md`) and not codified decisions (that's `DECISION_LOG.md`).

---

## Z-index doesn't resolve "same opaque pixels"

When two UI elements are anchored to the same spot and one has an opaque background (`bg-card/95`, `backdrop-blur`, whatever), no layering order makes both usable. Whichever sits on top, the other is visually hidden and unreachable. The upstream `ProviderCard` deliberately overlaid the hover-action strip on top of the quota pill — that was the design. Fighting it by promoting the pill above didn't fix the collision; it just flipped which element became unreachable.

**Rule:** Before reaching for `z-index`, describe where each element is anchored in the DOM and ask whether they occupy the same pixels. If yes, the fix is structural — put them side-by-side in the flex row, or swap one out for the other on state change. Stacking only helps when the elements occupy different pixels and the question is which lid sits above a sticky-out bit.

**How to apply:** When an "overlay on hover" design collides with an always-visible element, reach for `hidden group-hover:flex` on an in-flow sibling before reaching for `absolute + z-index`. The sibling approach never overlaps and never needs `bg-*` to hide the element behind it.

## I18n default language: pick English, not the developer's language

Fresh `settings.json` files don't have a `language` field. Whatever `unwrap_or("...")` you pick becomes the default for every user who hasn't explicitly set it. Upstream `cc-switch` defaulted to `"zh"` because that was the developer's language; Switchy inherited that default and shipped a Chinese tray menu to English users on fresh installs. Frontend i18n (react-i18next) has browser-language detection that masks this; Rust-side defaults (tray menu, system dialogs) do not, so they silently fall through to whatever the fallback arm is.

**Rule:** For any `match language { "en" => ..., "ja" => ..., _ => ... }` default arm in Rust code, the `_` should be English, not Chinese. Browser i18n detection doesn't reach Rust.

**How to apply:** When auditing i18n coverage, grep Rust for `language.as_deref().unwrap_or(` and `match .* language` — those fall-through arms are the ones end-users never see until they complain about a foreign-language UI element.

## Don't chain bug-fix attempts without tracing the failure each time

In the overlay-z-index thread, four commits tried to fix the same bug by moving things around (top-2 → bottom-2 → z-20-on-parent → z-20-on-child) without re-examining why each prior attempt failed. Each attempt rebuilt, reshipped, retested with the user — a 5-minute cycle four times over. The correct fix took one commit once the geometry was actually traced.

**Rule:** If the first attempt at a visual bug doesn't work, don't try a variant. Stop and ask: what does the DOM/CSS actually produce here, and why did the fix not take? One round of "let me look at the code again" beats three rounds of "let me try moving it."

**How to apply:** On any rebuild that takes >5 min (Tauri release builds, native compiles), the cost of being wrong compounds. Trace before you tweak.
