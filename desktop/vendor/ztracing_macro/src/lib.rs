//! `#[instrument]` as a pure passthrough: profiling stays out of the binary.

#[proc_macro_attribute]
pub fn instrument(
  _attr: proc_macro::TokenStream,
  item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
  item
}
