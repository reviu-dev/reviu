use super::*;

/// Secondary config selects collapse behind one trigger; primary concepts get dedicated chips.
pub(crate) fn selectable_config_options(options: &[SessionConfigOption]) -> Vec<ConfigSelector> {
  options
    .iter()
    .filter(|option| !config_option_is_primary_control(option))
    .filter_map(config_selector_from_option)
    .collect()
}

fn config_selector_from_option(option: &SessionConfigOption) -> Option<ConfigSelector> {
  let SessionConfigKind::Select(select) = &option.kind else {
    return None;
  };
  let values: Vec<(SessionConfigValueId, String, Option<String>)> = match &select.options {
    SessionConfigSelectOptions::Ungrouped(options) => options
      .iter()
      .map(|option| {
        (
          option.value.clone(),
          option.name.clone(),
          option.description.clone(),
        )
      })
      .collect(),
    SessionConfigSelectOptions::Grouped(groups) => groups
      .iter()
      .flat_map(|group| {
        group.options.iter().map(|option| {
          (
            option.value.clone(),
            option.name.clone(),
            option.description.clone(),
          )
        })
      })
      .collect(),
    _ => Vec::new(),
  };
  if values.is_empty() {
    return None;
  }
  let current_label = values
    .iter()
    .find(|(value, _, _)| *value == select.current_value)
    .map(|(_, name, _)| name.clone())
    .unwrap_or_else(|| option.name.clone());
  Some(ConfigSelector {
    id: option.id.clone(),
    name: option.name.clone().into(),
    current_value: select.current_value.clone(),
    current_label,
    values,
  })
}

pub(crate) fn model_config_selector(options: &[SessionConfigOption]) -> Option<ConfigSelector> {
  options
    .iter()
    .find(|option| matches!(option.category, Some(SessionConfigOptionCategory::Model)))
    .and_then(config_selector_from_option)
}

pub(crate) fn mode_config_selector(options: &[SessionConfigOption]) -> Option<ConfigSelector> {
  options
    .iter()
    .find(|option| {
      matches!(option.category, Some(SessionConfigOptionCategory::Mode))
        && !config_option_is_access(option)
    })
    .and_then(config_selector_from_option)
}

pub(crate) fn access_config_selector(options: &[SessionConfigOption]) -> Option<ConfigSelector> {
  options
    .iter()
    .find(|option| config_option_is_access(option))
    .and_then(config_selector_from_option)
}

pub(crate) fn reasoning_config_selector(options: &[SessionConfigOption]) -> Option<ConfigSelector> {
  options
    .iter()
    .find(|option| config_option_is_reasoning(option))
    .and_then(config_selector_from_option)
}

fn config_option_is_primary_control(option: &SessionConfigOption) -> bool {
  matches!(
    option.category,
    Some(
      SessionConfigOptionCategory::Model
        | SessionConfigOptionCategory::Mode
        | SessionConfigOptionCategory::ThoughtLevel
    )
  ) || config_option_is_reasoning(option)
    || config_option_is_access(option)
}

fn config_option_is_access(option: &SessionConfigOption) -> bool {
  if text_is_access_control(&option.name) || text_is_access_control(option.id.0.as_ref()) {
    return true;
  }

  let SessionConfigKind::Select(select) = &option.kind else {
    return false;
  };
  match &select.options {
    SessionConfigSelectOptions::Ungrouped(options) => options.iter().any(|option| {
      text_is_access_control(&option.name)
        || option
          .description
          .as_deref()
          .is_some_and(text_is_access_control)
    }),
    SessionConfigSelectOptions::Grouped(groups) => groups.iter().any(|group| {
      text_is_access_control(&group.name)
        || group.options.iter().any(|option| {
          text_is_access_control(&option.name)
            || option
              .description
              .as_deref()
              .is_some_and(text_is_access_control)
        })
    }),
    _ => false,
  }
}

fn config_option_is_reasoning(option: &SessionConfigOption) -> bool {
  matches!(
    option.category,
    Some(SessionConfigOptionCategory::ThoughtLevel)
  ) || text_is_reasoning_control(&option.name)
    || text_is_reasoning_control(option.id.0.as_ref())
}

pub(crate) fn modes_are_reasoning(modes: &[SessionMode]) -> bool {
  !modes.is_empty()
    && modes.iter().all(|mode| {
      text_is_reasoning_control(&mode.name) || text_is_reasoning_control(mode.id.0.as_ref())
    })
}

pub(crate) fn modes_are_access(modes: &[SessionMode]) -> bool {
  !modes.is_empty()
    && !modes_are_reasoning(modes)
    && modes.iter().any(|mode| {
      text_is_access_control(&mode.name)
        || text_is_access_control(mode.id.0.as_ref())
        || mode
          .description
          .as_deref()
          .is_some_and(text_is_access_control)
    })
}

fn text_is_access_control(text: &str) -> bool {
  let lower = text.trim().to_ascii_lowercase();
  if lower.contains("permission")
    || lower.contains("approval")
    || lower.contains("approve")
    || lower.contains("access")
    || lower.contains("bypass")
    || lower.contains("dangerous")
  {
    return true;
  }
  matches!(lower.as_str(), "accept edits" | "don't ask" | "dont ask")
}

fn text_is_reasoning_control(text: &str) -> bool {
  let lower = text.trim().to_ascii_lowercase();
  if lower.contains("reasoning") || lower.contains("thinking") || lower.contains("thought") {
    return true;
  }
  let normalized = lower
    .trim_start_matches("thinking:")
    .trim_start_matches("reasoning:")
    .trim()
    .replace([' ', '_', '-'], "");
  matches!(
    normalized.as_str(),
    "none" | "off" | "minimal" | "low" | "medium" | "normal" | "high" | "xhigh" | "max" | "ultra"
  )
}

pub(crate) fn normalize_reasoning_selector(mut selector: ConfigSelector) -> ConfigSelector {
  selector.current_label = reasoning_label(&selector.current_label);
  for (_, name, _) in &mut selector.values {
    *name = reasoning_label(name);
  }
  selector
}

pub(crate) fn reasoning_label(text: &str) -> String {
  let trimmed = text.trim();
  let stripped = strip_reasoning_prefix(trimmed).trim();
  match stripped.to_ascii_lowercase().as_str() {
    "none" => "None".to_string(),
    "off" => "Off".to_string(),
    "minimal" => "Minimal".to_string(),
    "low" => "Low".to_string(),
    "medium" => "Medium".to_string(),
    "normal" => "Normal".to_string(),
    "high" => "High".to_string(),
    "xhigh" => "Xhigh".to_string(),
    "max" => "Max".to_string(),
    "ultra" => "Ultra".to_string(),
    _ => stripped.to_string(),
  }
}

fn strip_reasoning_prefix(text: &str) -> &str {
  for prefix in [
    "thinking:",
    "reasoning:",
    "thought:",
    "thinking",
    "reasoning",
    "thought",
  ] {
    if text
      .get(..prefix.len())
      .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
    {
      return text.get(prefix.len()..).unwrap_or("");
    }
  }
  text
}

#[cfg(test)]
pub(crate) fn config_summary(selectors: &[ConfigSelector]) -> String {
  selectors
    .iter()
    .map(|selector| selector.current_label.as_str())
    .collect::<Vec<_>>()
    .join(" · ")
}

/// True once any option left the value the agent first advertised.
pub(crate) fn config_customized(
  selectors: &[ConfigSelector],
  defaults: &HashMap<SessionConfigId, SessionConfigValueId>,
) -> bool {
  selectors.iter().any(|selector| {
    defaults
      .get(&selector.id)
      .is_some_and(|default| *default != selector.current_value)
  })
}
