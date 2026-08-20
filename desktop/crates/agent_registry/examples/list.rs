fn main() {
  let registry = agent_registry::Registry::load();
  println!("source: {:?}", registry.source);
  for agent in registry.runnable() {
    let (program, args) = agent.command().expect("runnable");
    println!(
      "{:<22} {:<22} {} {}",
      agent.id.as_str(),
      agent.name,
      program,
      args.join(" ")
    );
  }
}
