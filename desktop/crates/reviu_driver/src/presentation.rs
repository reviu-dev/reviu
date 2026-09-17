use std::collections::{BTreeMap, HashMap};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, bail};
use gpui::App;
use gpui::http_client::{AsyncBody, FakeHttpClient, Response};
use workspace::DriverPresentation;

#[derive(Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Manifest {
  hide_profile_badge: bool,
  project_avatars: BTreeMap<PathBuf, PathBuf>,
  user_avatar: Option<PathBuf>,
}

#[derive(Default)]
pub(crate) struct Presentation {
  appearance: DriverPresentation,
  images: HashMap<String, Vec<u8>>,
}

impl Presentation {
  pub(crate) fn load(path: &Path) -> anyhow::Result<Self> {
    let path = path
      .canonicalize()
      .with_context(|| format!("Presentation manifest {}", path.display()))?;
    let directory = path
      .parent()
      .context("Presentation manifest has no parent")?;
    let manifest: Manifest = serde_json::from_slice(&std::fs::read(&path)?)
      .with_context(|| format!("Invalid presentation manifest {}", path.display()))?;
    let mut presentation = Self::default();
    presentation.appearance.hide_profile_badge = manifest.hide_profile_badge;
    for (index, (project, avatar)) in manifest.project_avatars.into_iter().enumerate() {
      let project = directory.join(project);
      let project = project
        .canonicalize()
        .with_context(|| format!("Project {}", project.display()))?;
      if !project.is_dir() {
        bail!("Project is not a directory: {}", project.display());
      }
      let uri = format!("https://reviu-driver.invalid/project/{index}.png");
      presentation
        .images
        .insert(uri.clone(), load_image(&directory.join(avatar))?);
      if presentation
        .appearance
        .project_avatars
        .insert(project, uri)
        .is_some()
      {
        bail!("Multiple avatars configured for the same canonical project");
      }
    }
    if let Some(avatar) = manifest.user_avatar {
      let uri = "https://reviu-driver.invalid/user.png".to_string();
      presentation
        .images
        .insert(uri.clone(), load_image(&directory.join(avatar))?);
      presentation.appearance.user_avatar = Some(uri);
    }
    Ok(presentation)
  }

  pub(crate) fn install(self, cx: &mut App) {
    let images = Arc::new(self.images);
    cx.set_http_client(FakeHttpClient::create(move |request| {
      let response = image_response(&images, &request.uri().to_string());
      async move { response }
    }));
    cx.set_global(self.appearance);
  }
}

fn load_image(path: &Path) -> anyhow::Result<Vec<u8>> {
  let bytes =
    std::fs::read(path).with_context(|| format!("Read presentation image {}", path.display()))?;
  let image = image::load_from_memory(&bytes)
    .with_context(|| format!("Decode presentation image {}", path.display()))?;
  let mut output = Cursor::new(Vec::new());
  image.write_to(&mut output, image::ImageFormat::Png)?;
  Ok(output.into_inner())
}

fn image_response(
  images: &HashMap<String, Vec<u8>>,
  uri: &str,
) -> anyhow::Result<Response<AsyncBody>> {
  match images.get(uri) {
    Some(bytes) => Ok(
      Response::builder()
        .status(200)
        .header("Content-Type", "image/png")
        .body(bytes.clone().into())?,
    ),
    None => Ok(Response::builder().status(404).body(AsyncBody::empty())?),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::atomic::{AtomicUsize, Ordering};

  struct Fixture(PathBuf);

  impl Fixture {
    fn new() -> Self {
      static NEXT: AtomicUsize = AtomicUsize::new(0);
      let path = std::env::temp_dir().join(format!(
        "reviu-presentation-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
      ));
      std::fs::create_dir_all(path.join("repo")).expect("fixture repo");
      image::DynamicImage::new_rgb8(2, 2)
        .save(path.join("avatar.png"))
        .expect("fixture image");
      Self(path)
    }

    fn load(&self, manifest: &str) -> anyhow::Result<Presentation> {
      let path = self.0.join("presentation.json");
      std::fs::write(&path, manifest)?;
      Presentation::load(&path)
    }
  }

  impl Drop for Fixture {
    fn drop(&mut self) {
      std::fs::remove_dir_all(&self.0).expect("remove fixture");
    }
  }

  #[test]
  fn local_manifest_resolves_paths_without_a_github_remote() {
    let fixture = Fixture::new();
    let presentation = fixture
      .load(
        r#"{
      "hide_profile_badge": true,
      "project_avatars": {"repo": "avatar.png"},
      "user_avatar": "avatar.png"
    }"#,
      )
      .expect("presentation");
    assert!(presentation.appearance.hide_profile_badge);
    let project = fixture
      .0
      .join("repo")
      .canonicalize()
      .expect("canonical project");
    let uri = presentation
      .appearance
      .project_avatars
      .get(&project)
      .expect("project avatar");
    assert_eq!(
      image_response(&presentation.images, uri)
        .expect("image response")
        .status(),
      200
    );
    let user_uri = presentation
      .appearance
      .user_avatar
      .as_ref()
      .expect("user avatar");
    assert!(presentation.images.contains_key(user_uri));
    assert_eq!(
      image_response(&presentation.images, "https://github.com/real-user.png")
        .expect("unknown image")
        .status(),
      404
    );
  }

  #[test]
  fn invalid_manifests_and_images_are_rejected() {
    let fixture = Fixture::new();
    for manifest in [
      r#"{"hide_profile_bagde":true}"#,
      r#"{"user_avatar":"missing.png"}"#,
      r#"{"user_avatar":"presentation.json"}"#,
      r#"{"project_avatars":{"missing":"avatar.png"}}"#,
      r#"{"project_avatars":{"avatar.png":"avatar.png"}}"#,
      r#"{"project_avatars":{"repo":"avatar.png","./repo":"avatar.png"}}"#,
    ] {
      assert!(
        fixture.load(manifest).is_err(),
        "accepted invalid manifest: {manifest}"
      );
    }
  }

  #[test]
  fn empty_manifest_does_not_override_appearance() {
    let presentation = Fixture::new().load("{}").expect("empty manifest");
    assert!(!presentation.appearance.hide_profile_badge);
    assert!(presentation.appearance.project_avatars.is_empty());
    assert!(presentation.appearance.user_avatar.is_none());
    assert!(presentation.images.is_empty());
  }
}
