#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReviuProLaunchOffer {
  pub badge: &'static str,
  pub title: &'static str,
  pub description: &'static str,
  pub launch_price: &'static str,
  pub regular_price: &'static str,
  pub billing_period: &'static str,
  pub checkout_cta: &'static str,
  pub github_upgrade_description: &'static str,
}

const STANDARD_GITHUB_UPGRADE_DESCRIPTION: &str = "Upgrade to Reviu Pro for $19/month to unlock GitHub notifications, repository browsing, pull request reviews, issues, and branch-to-PR shortcuts.";

const ACTIVE_REVIU_PRO_LAUNCH_OFFER: Option<ReviuProLaunchOffer> = None;

pub(crate) fn active_reviu_pro_launch_offer() -> Option<ReviuProLaunchOffer> {
  ACTIVE_REVIU_PRO_LAUNCH_OFFER
}

pub(crate) fn reviu_pro_checkout_cta_label() -> &'static str {
  active_reviu_pro_launch_offer()
    .map(|offer| offer.checkout_cta)
    .unwrap_or("Subscribe")
}

pub(crate) fn github_upgrade_description() -> &'static str {
  active_reviu_pro_launch_offer()
    .map(|offer| offer.github_upgrade_description)
    .unwrap_or(STANDARD_GITHUB_UPGRADE_DESCRIPTION)
}
