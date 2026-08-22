//! Where a comment written on a diff goes, and what the diff offers because of
//! it. One place decides the whole capability set, so the two review flows
//! cannot quietly drift apart as the editor grows knobs.

use std::collections::HashMap;

use editor::{
  Editor, ReviewCommentAssetUrlResolver, ReviewCommentCancelHandler, ReviewCommentCreateHandler,
  ReviewCommentDeleteHandler, ReviewCommentDisplayMode, ReviewCommentEditHandler,
  ReviewCommentImageUploadHandler, ReviewCommentLinkHandler, ReviewCommentPreviewRenderer,
  ReviewCommentResolveHandler, ReviewCommentSendHandler, ReviewCommentSuggestionActionFactory,
};
use gpui::{App, Entity};

/// Comments addressed to the agent. No threads and no resolution: they are
/// instructions, and a completed turn takes them away.
pub(crate) struct AgentReviewHandlers {
  pub create: ReviewCommentCreateHandler,
  pub edit: ReviewCommentEditHandler,
  pub delete: ReviewCommentDeleteHandler,
  pub cancel: ReviewCommentCancelHandler,
  pub send: ReviewCommentSendHandler,
}

/// Comments of a pull request review. They thread, they resolve, and they
/// outlive the session, so the card carries far more than the agent's does.
/// What a host cannot offer stays `None` rather than being faked.
pub(crate) struct GithubReviewHandlers {
  pub create: ReviewCommentCreateHandler,
  pub edit: ReviewCommentEditHandler,
  pub delete: ReviewCommentDeleteHandler,
  pub cancel: ReviewCommentCancelHandler,
  pub resolve: ReviewCommentResolveHandler,
  pub asset_url_resolver: ReviewCommentAssetUrlResolver,
  pub preview_renderer: Option<ReviewCommentPreviewRenderer>,
  pub link: Option<ReviewCommentLinkHandler>,
  pub image_upload: Option<ReviewCommentImageUploadHandler>,
  pub suggestion_action_factory: Option<ReviewCommentSuggestionActionFactory>,
}

pub(crate) enum ReviewDestination {
  Agent(Box<AgentReviewHandlers>),
  Github(Box<GithubReviewHandlers>),
  /// This editor takes no comments: the handlers go, and so does what was
  /// already on it.
  None,
}

/// Installs everything a destination offers and clears everything it does not.
pub(crate) fn configure_review(
  editor: &Entity<Editor>,
  destination: ReviewDestination,
  cx: &mut App,
) {
  editor.update(cx, |editor, cx| match destination {
    ReviewDestination::Agent(handlers) => {
      editor.set_review_comment_display_mode(ReviewCommentDisplayMode::LocalNote, cx);
      editor.set_review_comment_replies_enabled(false, cx);
      editor.set_review_comment_create_handler(Some(handlers.create), cx);
      editor.set_review_comment_edit_handler(Some(handlers.edit), cx);
      editor.set_review_comment_delete_handler(Some(handlers.delete), cx);
      editor.set_review_comment_cancel_handler(Some(handlers.cancel), cx);
      editor.set_review_comment_send_handler(Some(handlers.send), cx);
      // Nothing to resolve, no GitHub assets to fetch, no pull request to
      // belong to.
      editor.set_review_comment_resolve_handler(None, cx);
      editor.set_review_comment_link_handler(None, cx);
      editor.set_review_comment_image_upload_handler(None, cx);
      editor.set_review_comment_asset_url_resolver(None, cx);
      editor.set_review_comment_preview_renderer(None, cx);
      editor.set_review_comment_suggestion_action_factory(None, cx);
      editor.set_review_comment_pr_number(None, cx);
    }
    ReviewDestination::Github(handlers) => {
      editor.set_review_comment_display_mode(ReviewCommentDisplayMode::Conversation, cx);
      editor.set_review_comment_replies_enabled(true, cx);
      editor.set_review_comment_create_handler(Some(handlers.create), cx);
      editor.set_review_comment_edit_handler(Some(handlers.edit), cx);
      editor.set_review_comment_delete_handler(Some(handlers.delete), cx);
      editor.set_review_comment_cancel_handler(Some(handlers.cancel), cx);
      editor.set_review_comment_resolve_handler(Some(handlers.resolve), cx);
      editor.set_review_comment_asset_url_resolver(Some(handlers.asset_url_resolver), cx);
      editor.set_review_comment_preview_renderer(handlers.preview_renderer, cx);
      editor.set_review_comment_link_handler(handlers.link, cx);
      editor.set_review_comment_image_upload_handler(handlers.image_upload, cx);
      editor.set_review_comment_suggestion_action_factory(handlers.suggestion_action_factory, cx);
      // There is no agent on this side to send a comment to.
      editor.set_review_comment_send_handler(None, cx);
      editor.set_sendable_review_comment_ids(std::iter::empty::<u64>(), cx);
    }
    ReviewDestination::None => {
      editor.set_review_comment_display_mode(ReviewCommentDisplayMode::Conversation, cx);
      editor.set_review_comment_create_handler(None, cx);
      editor.set_review_comment_edit_handler(None, cx);
      editor.set_review_comment_delete_handler(None, cx);
      editor.set_review_comment_cancel_handler(None, cx);
      editor.set_review_comment_send_handler(None, cx);
      editor.set_review_comment_resolve_handler(None, cx);
      editor.set_review_comment_link_handler(None, cx);
      editor.set_review_comment_image_upload_handler(None, cx);
      editor.set_review_comment_asset_url_resolver(None, cx);
      editor.set_review_comment_preview_renderer(None, cx);
      editor.set_review_comment_suggestion_action_factory(None, cx);
      editor.set_review_comment_pr_number(None, cx);
      editor.set_editable_review_comment_ids(std::iter::empty::<u64>(), cx);
      editor.set_sendable_review_comment_ids(std::iter::empty::<u64>(), cx);
      editor.set_review_comments(Vec::new(), cx);
      editor.set_review_comment_code_reference_previews(HashMap::new(), cx);
    }
  });
}

#[cfg(test)]
mod tests {
  use super::*;
  use editor::ReviewCapabilities;
  use gpui::{AnyElement, AppContext as _, IntoElement as _, TestAppContext, div};
  use std::sync::Arc;

  fn agent_handlers() -> AgentReviewHandlers {
    AgentReviewHandlers {
      create: Arc::new(|_, _, _| {}),
      edit: Arc::new(|_, _, _, _| {}),
      delete: Arc::new(|_, _, _| {}),
      cancel: Arc::new(|_, _| {}),
      send: Arc::new(|_, _, _| {}),
    }
  }

  fn github_handlers() -> GithubReviewHandlers {
    GithubReviewHandlers {
      create: Arc::new(|_, _, _| {}),
      edit: Arc::new(|_, _, _, _| {}),
      delete: Arc::new(|_, _, _| {}),
      cancel: Arc::new(|_, _| {}),
      resolve: Arc::new(|_, _, _, _, _| {}),
      asset_url_resolver: Arc::new(|_| None),
      preview_renderer: Some(Arc::new(|_, _, _, _| -> AnyElement {
        div().into_any_element()
      })),
      link: Some(Arc::new(|_, _, _| false)),
      image_upload: Some(Arc::new(|_, _, _, _| {})),
      suggestion_action_factory: Some(Arc::new(|_, _, _, _| {
        Arc::new(|_, _| div().into_any_element())
      })),
    }
  }

  fn capabilities(destination: ReviewDestination, cx: &mut TestAppContext) -> ReviewCapabilities {
    let editor = cx.new(|cx| Editor::new_with_paths("/repo".into(), "src/main.rs".into(), cx));
    cx.update(|cx| configure_review(&editor, destination, cx));
    editor.read_with(cx, |editor, _| editor.review_capabilities())
  }

  #[gpui::test]
  fn the_agent_gets_a_note_it_can_send_and_nothing_from_github(cx: &mut TestAppContext) {
    let capabilities = capabilities(ReviewDestination::Agent(Box::new(agent_handlers())), cx);

    assert_eq!(
      capabilities,
      ReviewCapabilities {
        display_mode: ReviewCommentDisplayMode::LocalNote,
        replies_enabled: false,
        create: true,
        edit: true,
        delete: true,
        cancel: true,
        send: true,
        resolve: false,
        link: false,
        image_upload: false,
        asset_url_resolver: false,
        preview_renderer: false,
        suggestion_action_factory: false,
        pr_number: None,
      }
    );
  }

  #[gpui::test]
  fn github_gets_threads_and_resolution_but_no_send(cx: &mut TestAppContext) {
    let capabilities = capabilities(ReviewDestination::Github(Box::new(github_handlers())), cx);

    assert_eq!(
      capabilities,
      ReviewCapabilities {
        display_mode: ReviewCommentDisplayMode::Conversation,
        replies_enabled: true,
        create: true,
        edit: true,
        delete: true,
        cancel: true,
        // No agent on this side to send a comment to.
        send: false,
        resolve: true,
        link: true,
        image_upload: true,
        asset_url_resolver: true,
        preview_renderer: true,
        suggestion_action_factory: true,
        pr_number: None,
      }
    );
  }

  #[gpui::test]
  fn nothing_survives_a_teardown(cx: &mut TestAppContext) {
    let editor = cx.new(|cx| Editor::new_with_paths("/repo".into(), "src/main.rs".into(), cx));
    cx.update(|cx| {
      configure_review(
        &editor,
        ReviewDestination::Github(Box::new(github_handlers())),
        cx,
      );
      editor.update(cx, |editor, cx| {
        editor.set_review_comment_pr_number(Some(42), cx);
      });
      configure_review(&editor, ReviewDestination::None, cx);
    });

    let capabilities = editor.read_with(cx, |editor, _| editor.review_capabilities());
    assert_eq!(
      capabilities,
      ReviewCapabilities {
        display_mode: ReviewCommentDisplayMode::Conversation,
        replies_enabled: true,
        create: false,
        edit: false,
        delete: false,
        cancel: false,
        send: false,
        resolve: false,
        link: false,
        image_upload: false,
        asset_url_resolver: false,
        preview_renderer: false,
        suggestion_action_factory: false,
        pr_number: None,
      }
    );
  }
}
