fn main() {
  if workspace::handle_askpass_invocation() {
    return;
  }

  eprintln!("reviu-askpass must be launched by git or ssh");
  std::process::exit(1);
}
