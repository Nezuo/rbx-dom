# rbx_types Changelog

## Unreleased Changes

## 0.2.0 (2020-04-27)
* `Ref` can now represent null explicitly via `Ref::none` and `Ref::is_none`.
* Added `Region3` and `Region3int16` to `Variant` and `VariantType`.
* Added `legacy-compat` feature to enable conversions with rbx_dom_weak 1.x.

## 0.1.0 (2020-02-05)
* Initial release of types crate, should have rough feature parity with rbx_dom_weak.
* API will move a lot before becoming stable.