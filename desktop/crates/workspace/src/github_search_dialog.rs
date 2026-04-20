use std::sync::Arc;

use gpui::{App, AppContext as _, ParentElement, Styled, Window, px};
use ui::{
  GithubRepoSearchFn, GithubRepoSelectFn, GithubSearchPalette, GithubSearchPaletteConfig,
  GithubSearchRepoEntry, WindowExt,
};

use crate::{api::ApiClient, github_navigation::open_repo_target};

pub fn open_github_search_dialog(api: ApiClient, window: &mut Window, _cx: &mut App) {
  window.on_next_frame(move |window, cx| {
    open_github_search_dialog_inner(api.clone(), window, cx);
  });
}

fn open_github_search_dialog_inner(api: ApiClient, window: &mut Window, cx: &mut App) {
  let search_fn: GithubRepoSearchFn = {
    let api = api.clone();
    Arc::new(move |query: String| {
      let items = api.search_github_repositories(&query)?;
      Ok(
        items
          .into_iter()
          .map(|item| GithubSearchRepoEntry {
            owner: item.owner.into(),
            repo: item.name.into(),
            full_name: item.full_name.into(),
            description: item.description.map(Into::into),
            stars: item.stars,
            private: item.private,
            owner_avatar_url: item.owner_avatar_url.map(Into::into),
          })
          .collect(),
      )
    })
  };

  let on_select: GithubRepoSelectFn = Arc::new(|entry, _window, cx| {
    open_repo_target(
      entry.owner.to_string(),
      entry.repo.to_string(),
      None,
      None,
      None,
      cx,
    );
    Ok(())
  });

  let palette = cx.new(|cx| {
    GithubSearchPalette::new(
      window,
      cx,
      GithubSearchPaletteConfig::new(search_fn, on_select),
    )
  });
  let palette_for_dialog = palette.clone();

  window.open_dialog(cx, move |dialog, _, _| {
    dialog
      .p_0()
      .w(px(520.0))
      .border_0()
      .min_h_0()
      .overlay_closable(true)
      .keyboard(true)
      .close_button(false)
      .child(palette_for_dialog.clone())
  });
}
