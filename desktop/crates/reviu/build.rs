fn main() {
  #[cfg(target_os = "windows")]
  {
    embed_resource::compile("assets/reviu.rc", embed_resource::NONE)
      .manifest_required()
      .unwrap();
  }
}
