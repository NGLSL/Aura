# Local iced_tiny_skia 0.14.1 patch

Source: the published [`iced_tiny_skia` 0.14.1 crate](https://crates.io/crates/iced_tiny_skia/0.14.1),
copied without Cargo registry cache metadata. `Cargo.toml` and `src/` retain the
upstream crate layout and dependency versions. `LICENSE` is the MIT license from
the [official Iced source](https://github.com/iced-rs/iced/blob/0.14.0/LICENSE).

Aura uses the CPU renderer. The vector pipeline rasterizes SVGs at physical
resolution and uses the full transform to place the result. This preserves
rotation and translation along with high-DPI sizing, tint, opacity and clipping.
See [upstream PR #2954](https://github.com/iced-rs/iced/pull/2954) for the DPI fix.

`src/svg_dpi_tests.rs` exercises the real `Renderer::draw` offscreen pixel path at
100%, 125%, 150%, 175%, and 200%, including nonzero positions, rotation, tint,
opacity, and layer clipping. To run the tests from the Aura workspace, use the
renderer features resolved in its lockfile.

```powershell
cargo test --locked -p iced_tiny_skia --lib
```

Remove this directory and the root `[patch.crates-io]` entry when Aura upgrades to
an upstream renderer version that passes these DPI and rotation regressions.
