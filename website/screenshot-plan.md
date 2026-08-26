# Plan de capture pour la landing 1.0 (#535)

Session à dérouler sur le Mac. La landing est déjà branchée sur les nouveaux noms de fichiers : remplacer chaque PNG par la vraie capture, même nom, et c'est fini.

## Identité de démo

Tout est anonyme, rien ne montre joris-gallot :

- Auteur git : `Ava Collins <ava@lumenware.dev>` (config locale du repo de démo, posée par le seed).
- Deuxième auteur dans l'historique : `Noah Fischer <noah@lumenware.dev>`.
- Projet : `@lumenware/checkout`, une petite lib TS de panier/pricing.

Pour les shots Pro (PR, inbox), le nom affiché vient de GitHub : soit créer un compte/org de test (ex. org `lumenware-dev`, compte `ava-collins`), soit assumer le crop. Le seed ne peut pas le faire à ta place.

## Préparation

1. Seed du repo de démo :
   ```
   ./website/scripts/seed-demo-repo.sh ~/demo/checkout
   ```
   Ce qu'il pose : historique propre à deux auteurs sur `main`, branche courante `feature/checkout-discounts` avec working tree sale (discounts.ts modifié non stagé, tests stagés, promo-codes.ts untracked), `feature/messy-history` (5 commits wip pour le rebase interactif), `feature/tax-rounding` (conflit sur `src/pricing.ts` en mergeant `main`), une entrée de stash.
2. Lancer Reviu (profil dev ok, aucune donnée perso visible), ouvrir `~/demo/checkout`.
3. Créer les sessions agent, dans cet ordre (la plus récente en haut) :
   - « Write property tests for cart totals »
   - « Fix tax rounding for CHF and JPY »
   - « Add percentage discounts that stack with flat discounts » (celle-ci active, laisser l'agent bosser pour une vraie conversation)
4. Fenêtre : ~1510x945 points, capture Retina 2x (cible ~3020x1890, comme les fichiers actuels). Pas de plein écran, pas d'ombre système (capture de fenêtre sans ombre ou crop).
5. Chaque shot en clair ET en sombre (`_light` / `_dark`).

## Les 4 captures (x2 thèmes)

| Fichier | Vue | État à montrer |
|---|---|---|
| `hero_*.png` | Diff au centre + dock Changes | `src/discounts.ts` ouvert en diff, un commentaire de review inline posé sur le calcul du pourcentage (ex. « percentage should apply to the remaining amount, not the original subtotal »), dock à droite avec staged/unstaged/untracked visibles |
| `sessions_*.png` | Conversation au centre | Les 3 sessions dans la sidebar (statuts visibles si dispo), conversation de la session discounts avec tool calls de l'agent, composer en bas |
| `git_*.png` | Rebase interactif au centre | `git checkout feature/messy-history` (stash le working tree avant : `git stash -u`), lancer Interactive rebase depuis la palette sur les 5 commits wip ; alternative si moins lisible : le conflit (`feature/tax-rounding`, merge `main`, header Accept current/incoming) |
| `pr_*.png` | Diff + dock Pull request | La PR de `feature/checkout-discounts` ouverte dans l'onglet PR du dock : description, checks, threads ; nécessite la partie GitHub ci-dessous |

Remettre l'état hero après le shot git : `git checkout feature/checkout-discounts && git stash pop`.

## Partie GitHub (shot `pr_*` + inbox)

```
cd ~/demo/checkout
gh repo create lumenware-dev/checkout --private --source . --push
git commit -m "feat: percentage discounts with stacking"   # committer l'état hero le temps de la PR
git push -u origin feature/checkout-discounts
gh pr create --title "Add percentage discounts with stacking" \
  --body "Adds percentage discounts that stack with flat ones, plus promo code resolution. LAUNCH10 and WELCOME5 seeded."
```

Puis dans Reviu : brancher GitHub (Pro), ouvrir la PR depuis le bouton du header. Pour l'inbox : une review ou un commentaire depuis le deuxième compte pour générer une notification. Après capture, `git reset --soft HEAD~1` pour retrouver le working tree hero.

## Après les captures

- Remplacer les 8 PNG dans `website/src/assets/app_screenshots/` (mêmes noms).
- Regénérer `public/og.png` à partir du nouveau hero (1200x630).
- Les anciens `github_home_*`, `github_pr_conv_*`, `github_pr_changes_*` restent : le blog les importe encore.
- Vidéo hero : tâche #632, même seed, même scénario ; à capturer dans la même session si le temps le permet.
