# Patched crates.io copies

Opening Word, Excel and PowerPoint files goes through `office2pdf` → `typst` → `typst-svg` →
`hayro-svg`, and `typst-pdf`, which are built against crates.io `hayro-interpret` and `krilla`.
This workspace patches both to craft-libs' copies, which add enum variants and settings, so these are
copied here with the smallest changes that build against it. Everything else is unchanged.

- `hayro-svg` 0.7.0 (Apache-2.0 OR MIT, the hayro authors): `src/glyph.rs` skips
  `GlyphDrawMode::Clip` like `Invisible`.
- `typst-svg` 0.15.1 (Apache-2.0, the Typst authors): `src/image.rs` fills the extra
  `InterpreterSettings` fields with their defaults.
- `typst-pdf` 0.15.1 (Apache-2.0, the Typst authors): `src/convert.rs` fills the extra
  `SerializeSettings` fields with their defaults and reports krilla's extra errors generically.

Licence files are in each crate's folder.
