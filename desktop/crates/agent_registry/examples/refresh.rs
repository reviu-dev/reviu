fn main() {
  for attempt in 1..=2 {
    match agent_registry::refresh_global_blocking() {
      Ok(outcome) => println!("attempt {attempt}: {outcome:?}"),
      Err(err) => println!("attempt {attempt}: failed: {err}"),
    }
  }
}
