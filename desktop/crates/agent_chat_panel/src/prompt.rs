//! Prompt assembly: repo file listing, mentions resolution, content blocks.

use super::*;

pub(crate) async fn list_repo_files(cwd: PathBuf) -> Vec<String> {
  let output = async_process::Command::new("git")
    .args(["ls-files", "--cached", "--others", "--exclude-standard"])
    .current_dir(&cwd)
    .output()
    .await;
  match output {
    Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
      .lines()
      .map(|line| line.trim().to_string())
      .filter(|line| !line.is_empty())
      .take(MAX_REPO_FILES)
      .collect(),
    _ => Vec::new(),
  }
}

pub(crate) async fn run_git(cwd: &Path, args: &[&str]) -> anyhow::Result<String> {
  let output = async_process::Command::new("git")
    .args(args)
    .current_dir(cwd)
    .output()
    .await?;
  if !output.status.success() {
    anyhow::bail!(
      "git {} failed: {}",
      args.join(" "),
      String::from_utf8_lossy(&output.stderr).trim()
    );
  }
  Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub(crate) async fn detect_base_ref(cwd: &Path) -> Option<String> {
  if let Ok(out) = run_git(cwd, &["rev-parse", "--abbrev-ref", "origin/HEAD"]).await {
    let reference = out.trim();
    if !reference.is_empty() && reference != "origin/HEAD" {
      return Some(reference.to_string());
    }
  }
  for candidate in ["origin/main", "origin/master", "main", "master"] {
    if run_git(cwd, &["rev-parse", "--verify", "--quiet", candidate])
      .await
      .is_ok()
    {
      return Some(candidate.to_string());
    }
  }
  None
}

/// Build the ACP content blocks for a submitted message: the typed text, plus a `ResourceLink` per
/// `@file` mention and an embedded diff `Resource` per `@diff`/`@staged`/`@branch` mention.
pub(crate) async fn build_prompt_blocks(
  text: String,
  files: Arc<Vec<String>>,
  selection: Option<SelectionContext>,
  images: Vec<std::sync::Arc<gpui::Image>>,
  cwd: PathBuf,
) -> Vec<ContentBlock> {
  use base64::Engine as _;
  let mentions = mention::resolve_mentions(&text, files.as_slice(), selection.is_some());
  let mut blocks = vec![ContentBlock::Text(TextContent::new(text))];
  for image in images {
    let data = base64::engine::general_purpose::STANDARD.encode(&image.bytes);
    blocks.push(ContentBlock::Image(
      agent_client_protocol::schema::ImageContent::new(data, image.format.mime_type()),
    ));
  }

  for mention in mentions {
    match mention {
      ResolvedMention::File(path) => {
        let uri = format!("file://{}", cwd.join(&path).display());
        blocks.push(ContentBlock::ResourceLink(ResourceLink::new(path, uri)));
      }
      ResolvedMention::Diff(diff) => {
        let (kind, content) = resolve_diff(diff, &cwd).await;
        let resource = TextResourceContents::new(content, format!("reviu-diff://{kind}"))
          .mime_type(Some("text/x-diff".to_string()));
        blocks.push(ContentBlock::Resource(EmbeddedResource::new(
          EmbeddedResourceResource::TextResourceContents(resource),
        )));
      }
      ResolvedMention::Selection => {
        if let Some(selection) = selection.as_ref() {
          let uri = format!("reviu-selection://{}", selection.path);
          let resource = TextResourceContents::new(selection.text.clone(), uri);
          blocks.push(ContentBlock::Resource(EmbeddedResource::new(
            EmbeddedResourceResource::TextResourceContents(resource),
          )));
        }
      }
    }
  }

  blocks
}

pub(crate) async fn resolve_diff(diff: DiffMention, cwd: &Path) -> (&'static str, String) {
  match diff {
    DiffMention::Working => ("working", diff_text(run_git(cwd, &["diff", "HEAD"]).await)),
    DiffMention::Staged => (
      "staged",
      diff_text(run_git(cwd, &["diff", "--cached"]).await),
    ),
    DiffMention::Branch => match detect_base_ref(cwd).await {
      Some(base) => (
        "branch",
        diff_text(run_git(cwd, &["diff", &format!("{base}...HEAD")]).await),
      ),
      None => ("branch", "(could not determine base branch)".to_string()),
    },
  }
}

pub(crate) fn diff_text(diff: anyhow::Result<String>) -> String {
  match diff {
    Ok(diff) if diff.trim().is_empty() => "(no changes)".to_string(),
    Ok(diff) => mention::truncate_diff(&diff),
    Err(err) => format!("(error: {err})"),
  }
}
