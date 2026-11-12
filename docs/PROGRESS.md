# Journal de progression

Une entrée par session de travail, la plus récente en haut. Chaque session lit ce journal et `docs/ROADMAP.md` avant de commencer.

## 2026-11-12, matin — session autonome (R1 terminé sauf points manuels, début de R2)

**Fait (commits)**
- `c2c6ac7` Réglages mémorisés : fournisseur, modèle local, modèle OpenAI, modèle de vision (`settings`, `settings.models.changed`). L’accord d’envoi au cloud n’est jamais mémorisé.
- `b988977` Arrêt global « Stoppe toutes les tâches » (barre latérale) : `idle` / `stopped` / `requested` puis confirmé par `task.cancelled` ; une action en cours n’est jamais coupée, aucune autre ne démarre ensuite. Événements `task.started` (fournisseur), `task.progress` (`step:N/8`), `model.provider.unavailable`.
- `14f9049`, `c462411` Rétention de l’historique (90 jours par défaut, purge au démarrage), effacement des sessions passées avec confirmation, export JSON du journal dans `%LOCALAPPDATA%\app.aiwin11.desktop\exports`.
- `10ab10a` Invariant « rien de sensible dans SQLite » : garde `journal_code` (tout texte libre devient `REDACTED`), test du schéma colonne par colonne, scan du fichier de base.
- `d77fbc9` Qualité : ESLint (règles typées) + Prettier, ruff + mypy strict (`.venv`, `services/agent/requirements-dev.txt`), `cargo fmt` (`rustfmt.toml`, 120 colonnes) ; tout est dans `scripts/verify.ps1`. Reformatage mécanique des sources.
- `0a0bb5a` Mesures (`scripts/measure.ps1`, test `measure_permission_decision_latency`) : décision de permission p95 7,1 ms ; mémoire privée au repos 178–182 Mo (9 processus) ; fenêtre créée en 2,9 s à froid.
- `2528ade` R2 : manifeste formel par outil (`manifest.rs` : risque, dry-run, réversibilité, délai, capacités) ; les invariants de politique en dérivent ; fiche affichée dans **Sécurité**.

**Vérifié** : `scripts/verify.ps1 -Live` sur la machine de référence : tout passe (ruff, mypy, ESLint, Prettier, build, fmt, .NET, 68 tests Rust, clippy). Tests réels : 10/11 au premier passage ; `describes_the_previous_window_with_the_local_vision_model_after_approval` a échoué une fois (le modèle n’a pas proposé l’outil sur la Table des caractères) puis est passé à la relance — non-déterminisme du modèle, à surveiller. Build Tauri release OK.

**Non vérifié** : l’interface n’a pas été testée à la main (nouveau bouton d’arrêt, section Historique, fiche des outils) ; arrêt global « demandé » seulement en test unitaire ; T-003 (OpenAI réel) toujours sans clé.

**Problèmes** : heredocs bash avec du Python contenant `\a` ou des apostrophes : écrire les scripts de modification dans le scratchpad puis `python fichier.py`.

**Suite de la session (R2)**
- `8740de6` Autorisations temporaires et révocables (migration 003, table `grants`, section **Sécurité**) ; relues à chaque évaluation, elles ne transforment qu’un « Demander » en « Autoriser » (jamais pour une action critique, la lecture d’écran, les données privées, la simulation ni le cloud).
- `d8c8f24` Outil `list_processes` (lecture seule) de bout en bout ; test réel OK (pont + qwen3:8b).
- `88769b7` Outil `get_system_status` (mémoire, processeur, disques, durée de fonctionnement) ; scénario « PC lent » en lecture seule testé en réel.
- `6b877d5` Presse-papiers : `read_clipboard` (règle `private_data`, toujours demandé) et `write_clipboard` (critique, irréversible). Lecture réelle testée ; **écriture réelle non testée** (test opt-in `AIWIN11_TEST_CLIPBOARD=1`, pour ne pas écraser le presse-papiers de l’utilisateur).
- `5b29b07` Fenêtres : `list_windows` (données d’écran, toujours demandé) et `arrange_window` (premier plan, réduire, restaurer, agrandir une fenêtre de la liste de la même demande ; jamais fermer). Tests réels OK sur une Table des caractères ouverte par le test.

- `93edbed` Outil `list_services` (lecture seule, services automatiques arrêtés d’abord) ; NuGet `System.ServiceProcess.ServiceController` 10.0.2 épinglé.

**Vérification finale de la session** (`scripts/verify.ps1 -Live -Tauri`, machine de référence, Ollama portable `qwen3:8b` + `gemma3:4b`) : tout passe — ruff, mypy strict, ESLint, Prettier, build de l’interface, `cargo fmt --check`, pont .NET, 80 tests Rust, clippy, **19/19 tests réels**, build Tauri release. Aucun Bloc-notes de l’utilisateur n’était ouvert ; aucune application de l’utilisateur n’a été fermée ni modifiée (les tests de fenêtres agissent sur une Table des caractères qu’ils ouvrent eux-mêmes).

**Toujours non vérifié** : interface jamais testée à la main (arrêt global, Historique, autorisations temporaires, fiches d’outils, nouvelles approbations) ; écriture réelle du presse-papiers (opt-in) ; OpenAI réel (T-003) ; lecteur d’écran.

**Problèmes connus** : le premier appel au pont après `dotnet publish` peut dépasser le délai (démarrage à froid) — relancer ; un test de vision a échoué une fois par non-déterminisme du modèle ; `cargo fmt` reformate les imports, donc les scripts de modification doivent viser des ancres stables.

**Prochaine étape** : voir l’entrée suivante (en haut) ou, si aucune : R2, politiques par ressource (dossiers autorisés au-delà du dossier dédié, applications autorisées), ApplicationService (liste fermée d’applications), lecture des services Windows, PowerShell approuvé (liste fermée), puis l’assistant élevé (option B) — à faire **avec l’utilisateur présent** : il exige une revue explicite (AGENTS.md) et l’invite UAC ne peut pas être validée sans lui.

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
