# Journal de progression

Une entrée par session de travail, la plus récente en haut. Chaque session lit ce journal et `docs/ROADMAP.md` avant de commencer.

## 2026-11-12, 01h15 — préparation du travail autonome

**Fait**
- Feuille de route complète (`docs/ROADMAP.md`) avec toutes les décisions prises : dépôt local, installateur classique, pas de chiffrement de la base (rien de sensible stocké), machine de référence = PC de l’utilisateur, actions administrateur via un assistant élevé à la demande (option B, en R2).
- `scripts/verify.ps1` : lance toutes les vérifications locales (agent Python, interface, pont .NET, tests Rust, clippy ; `-Live` pour les tests réels, `-Tauri` pour le build release). Remplace la CI puisque le dépôt est local.
- Reprise automatique programmée à 03h45 (tâche `aiwin11-reprise-roadmap`).

**État vérifié** (dernière vérification complète, 12/11 vers 00h30) : 10 tests Python, 59 tests Rust, clippy, interface, pont .NET et build Tauri release passent ; 11 tests réels passent avec Ollama `qwen3:8b` et `gemma3:4b`.

**Prochaine étape** : phase R1 de la feuille de route, dans cet ordre :
1. Réglages mémorisés (fournisseur, modèle, modèle de vision) dans la table `settings`.
2. Arrêt global « Stoppe toutes les tâches » (état demandé / confirmé).
3. Événements `task.started`, `task.progress`, `model.provider.unavailable`.
4. Rétention, effacement et export du journal.
5. Test garantissant que rien de sensible n’est écrit dans SQLite.
6. ESLint/Prettier, ruff/mypy, `cargo fmt --check`, intégrés à `scripts/verify.ps1`.
7. Mesures de performance sur la machine de référence.
