fn main() {
  // Compile dockerfile and dart grammars directly so they link against
  // the same tree-sitter 0.25 runtime as all other grammars.
  // The crate versions of these grammars depend on tree-sitter 0.20,
  // which causes duplicate C symbol conflicts on Linux.

  for name in ["dockerfile", "dart"] {
    let dir = format!("{}/grammars/{name}", env!("CARGO_MANIFEST_DIR"));

    let mut build = cc::Build::new();
    build
      .include(&dir)
      .file(format!("{dir}/parser.c"))
      .file(format!("{dir}/scanner.c"))
      .warnings(false);
    build.compile(&format!("tree-sitter-{name}"));
  }
}
