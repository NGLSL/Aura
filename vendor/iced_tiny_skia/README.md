# Local iced_tiny_skia 0.13.0 patch

Source: the published [`iced_tiny_skia` 0.13.0 crate](https://crates.io/crates/iced_tiny_skia/0.13.0),
copied without Cargo registry cache metadata. `Cargo.toml` and `src/` retain the
upstream crate layout and dependency versions. `LICENSE` is the MIT license from
the [official Iced 0.13.0 source](https://github.com/iced-rs/iced/blob/0.13.0/LICENSE).

Aura keeps the CPU renderer. This patch fixes SVG positions and dimensions being
scaled twice at Windows DPI scales above 100%. The engine passes logical bounds;
the vector pipeline rasterizes at physical resolution and transforms the origin
once, retaining rotation, translation, tint, opacity, and clipping. Raster image
rendering is unchanged. See also [upstream PR #2954](https://github.com/iced-rs/iced/pull/2954).

`src/svg_dpi_tests.rs` exercises the real `Renderer::draw` offscreen pixel path at
100%, 125%, 150%, 175%, and 200%, including nonzero positions, rotation, tint,
opacity, and layer clipping. Run from the Aura workspace:

```powershell
cargo test -p iced_tiny_skia --lib
```

Remove this directory and the root `[patch.crates-io]` entry when Aura upgrades to
an upstream renderer version that passes these DPI and rotation regressions.
