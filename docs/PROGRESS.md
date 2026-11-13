# Journal de progression

Une entrée par session de travail, la plus récente en haut. Chaque session lit ce journal et `docs/ROADMAP.md` avant de commencer.

## Recette UI avec profil isolé

- Mémoire de conversation validée avec Ollama local : un code synthétique est rappelé lors d’un suivi puis oublié après **Nouvelle conversation** (`-SessionMemory`). Déclencheur de dossier corrigé : l’empreinte de référence est sauvegardée lors de l’activation, pour détecter un ajout avant le premier passage du planificateur. Test Rust et recette release réussis, avec rapport visible (`-FolderTrigger`). Suites standard : 161 tests du cœur Rust et 16 tests d’interface réussis.

- MCP validé dans la release avec Ollama local et un serveur stdio de recette. Sonde (version et outil), liste et appel avec accord confirmés. Correction du bouton **Tester**, qui refuse maintenant de lancer un serveur modifié et actualise l’état pour afficher **Accepter la nouvelle version**. Nouvelle sonde validée après acceptation.

- Diagnostic PowerShell prédéfini de l’édition et du build Windows validé avec Ollama local. Les deux scénarios de la release vérifient dans le journal l’exécution après accord et l’absence d’exécution après refus (`-PowerShellDiagnostic Approve|Decline`).

- Mémoire : indexation, recherche par contenu et retrait d’un extrait synthétique validés dans la release (`scripts/smoke-desktop.ps1 -MemoryIndex`). Workflows : deux étapes terminées et une condition non satisfaite dans une demande locale soumise depuis l’interface (`-WorkflowCondition`). La recherche sémantique par l’agent, la nouvelle tentative sur erreur et le déclenchement sur changement de dossier restent à valider.

- Suite de la recette : mode simple/expert, fenêtre flottante et actualisation du tableau de bord validés. Ctrl+Alt+Retour arrière annule une tâche en attente d’accord sans exécuter l’action, comme le confirme le journal. L’arrêt pendant une action déjà engagée reste à essayer.

- Release compilée avec l’identifiant `app.aiwin11.recette`. Ollama strictement local, sans changement du profil courant. Écrans retournés par le broker, ouverture du Bloc-notes approuvée et auditée, refus sans exécution et arrêt pendant une approbation validés. UI Automation confirme le focus du lien d’évitement et du bouton de refus. Les annonces par lecteur d’écran restent à écouter.
- Palette (raccourci, Échap, transfert de brouillon) à 40 ms au p95 sur 22 ouvertures. Catalogue SAPI validé, sans ouverture du microphone.

## 2026-11-13, recette utilisateur locale

- Ollama portable lancé en mode strictement local avec les modèles déjà installés. 21 des 23 essais interactifs ont réussi, dont le lancement approuvé du Bloc-notes, les opérations sur fichiers de test, le diagnostic système et la liste des écrans. Les deux essais ciblant la Table des caractères ont refusé l’inspection quand une autre fenêtre est passée devant. Le test de l’interface a été interrompu avant l’envoi, car le fournisseur sélectionné dans le profil courant était cloud.
- MCP : sonde et appel en lecture seule réussis sur un processus Python local de test via stdio. Test reproductible avec `cargo test --locked -p aiwin11-core probes_and_calls_a_read_only_mcp_process_over_stdio -- --ignored` après avoir renseigné `AIWIN11_PYTHON`. La recette dans l’interface avec un serveur configuré reste à faire.
- 161 tests Rust du cœur et 11 tests gRPC réussis. Le micro, les moteurs de génération image/vidéo, la mémoire avec embeddings et la recette interactive des workflows et agents personnalisés restent à essayer.

## 2026-11-13, recette de l’installateur

- Installation NSIS dans un dossier de test sur le poste Windows de référence. Python embarqué 3.13.3 avec SQLite, premier inventaire des écrans via le broker, catalogue SAPI et palette validés sur l’exécutable installé. Palette à 38 ms au p95 sur 22 ouvertures. Désinstallation vérifiée, données applicatives conservées.
- L’inventaire d’écrans dispose de 20 s pour inclure le démarrage du pont autonome et d’une réponse bornée à 16 Kio. Le journal distingue un délai signalé par le pont d’une absence de réponse. L’ancien échec intermittent reste à reproduire sur un poste vierge.

## 2026-11-13, validation Windows après reprise

- `scripts/verify.ps1 -Ipc -Tauri` au vert sur le poste Windows. 161 tests du cœur Rust, 11 tests gRPC Rust, 28 tests Python et 16 tests Vitest. Compilation du pont et de l’assistant .NET sans avertissement.
- Suite interactive avec Ollama en mode local strict : 22 tests réussis au premier passage. Les deux scénarios repris réussissent après correction du test : recherche ou lecture de fichiers auditée, et remise au premier plan de la fenêtre de test avant chaque demande d’inspection.
- Release Windows : la palette passe le test UI Automation (39 ms au p95 sur 22 ouvertures). La liste d’écrans a échoué sur les premiers essais de l’Inspecteur (`broker.windows.failed`), puis le scénario isolé a réussi en passant par le broker. Le comportement au premier appel à froid reste à reproduire.
- Installateur NSIS par utilisateur généré avec les deux composants .NET autonomes et Python 3.13 embarqué. Le script valide le SHA-256 du paquet CPython dans le manifeste SPDX, y compris lorsque l’archive est déjà en cache. L’essai d’installation sur un poste vierge et la mise à jour signée restent à faire.

## 2026-11-13, point « Phases R1–R9 (§5) »

- Audit complet et mise à jour des statuts de la section §5 (`R1` à `R9`) de `docs/ROADMAP.md` pour refléter l’ensemble des fonctionnalités et tests implémentés (`R2`–`R9` étaient encore partiellement sans pastilles de statut), tout en conservant `🟡` / `❌` / `🔷` sur les éléments restants (ex. démarrage à froid Windows, test multi-DPI matériel, bundle NSIS/MSI désactivé dans `tauri.conf.json`, déclencheurs OS proactifs).

## 2026-11-13, point « Exigences non fonctionnelles (§4) »

- Vérification de tous les critères de la section §4 de `docs/ROADMAP.md` : 9/10 exigences déjà validées (`✅`), dont l’isolation de crash agent (`crashing_agent_process_fails_cleanly_and_leaves_core_healthy`) et le mode local strict sans requête cloud (`private_mode_blocks_cloud_routing_and_fallback_before_network`).
- `Interface prête < 3 s` reste honnêtement en `🟡` (333–340 ms mesurés à chaud ; la mesure après démarrage à froid complet de Windows nécessite un redémarrage OS).

## 2026-11-13, point « Scénarios et matrice de tests (§3) »

- T-011 (`services/core/src/task.rs`) : ajout de `private_mode_blocks_cloud_routing_and_fallback_before_network` prouvant qu’en mode privé aucun appel cloud (`cloud_proposer`) n’est émis (`PRIVATE_MODE` + `model.cloud.denied`).
- T-010 (`services/core/src/task.rs`, `services/agent/test_agent.py`) : ajout de `prompt_injection_in_observations_and_mcp_outputs_are_framed` et `test_rejects_multi_tool_smuggling_and_injected_approval` (encadrement `<<<DEBUT ... FIN>>>`, rejet du multi-outil furtif et des arguments `approved` injectés).
- T-013 (`services/core/src/windows_bridge.rs`) : ajout de `display_list_accepts_negative_secondary_coordinates_and_rejects_zero_dimensions` pour la validation multi-écrans.
- `docs/ROADMAP.md` §3 actualisé.

## 2026-11-13, point « Compléments §2.5 »

- Limites d’agent (`services/core/src/task.rs`, `apps/desktop/src/CustomAgents.tsx`, `main.tsx`) : `MAX_AGENT_STEPS` relevé à `30` et ajout de `MAX_TOOL_CALLS = 50` (`AGENT_TOOL_LIMIT` + événement `task.limit.reached`).
- Résilience (`services/core/src/task.rs`) : ajout du test d’injection de panne `crashing_agent_process_fails_cleanly_and_leaves_core_healthy` vérifiant qu’un agent Python en échec ne bloque ni le cœur ni les exécutions suivantes.
- Banc de performance (`apps/desktop/src/ModelProbe.tsx`) : ajout du bouton « Banc de performance » pour sonder séquentiellement les modèles locaux et comparer latence, débit, VRAM et capacités dans le tableau.
- `docs/ROADMAP.md` §2.5 actualisé.

## 2026-11-13, point « Transmission et règles du dépôt (§2.4) »

- Analyseurs .NET activés (`EnableNETAnalyzers=true`, `AnalysisLevel=latest-recommended`) dans `services/windows/AiWin11.Windows.csproj` et `services/admin/AiWin11.Admin.csproj` avec `TreatWarningsAsErrors=true`.
- Correction des 12 alertes d’analyse (`CA1806`, `CA1838`) dans `services/windows/` (`WindowCapture.cs`, `WindowInspector.cs`, `Notepad.cs`, `CharacterMapInput.cs`, `ControlInput.cs`, `NotepadInput.cs`) en remplaçant les P/Invoke `StringBuilder` par des tampons `char[]` et en vérifiant/ignorant explicitement le retour de `ReleaseDC`.
- `docs/ROADMAP.md` §2.4 actualisé avec les compteurs vérifiés (170 tests Rust, 27 tests Python, 16 tests Vitest) et l’état d’accessibilité / badges local-cloud.

## 2026-11-13, point « Spécification v1.0 (§2.3) »

- `export_policy_json` 🧪 (`yaml_config.rs`, `main.rs`, `Security.tsx`) : export JSON structuré de la politique active (sujet, actions, risques, ressources protégées, contraintes).
- `AiProfile` 🧪 (`lib.rs`, `main.rs`, `Routing.tsx`) : profil IA persistant (`strategy`, `requireTools`, `requireVision`, `allowCloudFallback`, `allowScreenUpload`, `allowDocumentUpload`, `taskBudgetCents`) refusant tout envoi cloud en mode privé.
- Détection d’élévation Win32 🧪 (`services/windows/KeyInput.cs`, `Program.cs`) : `IsProcessElevated` identifie explicitement les fenêtres administrateur (`window_elevated`) au lieu de tenter un contournement.
- `docs/ROADMAP.md` §2.3 actualisé.

## 2026-11-13, point « Cadrage v0.1 (§2.2) »

- `yaml_config.rs` 🧪 (`services/core/src/yaml_config.rs`, `apps/desktop/src-tauri/src/main.rs`, `Security.tsx`) : export et import YAML validés par schéma v1 (`execution_mode`, `simulate`, `privacy_mode`, `local_strategy`, `agent_models`, `require_approval_for`) avec test unitaire d’aller-retour et rejet des versions/clés inconnues.
- Simulateur de politique 🧪 (`Security.tsx`) : simulateur interactif des décisions (`Autorisé`, `Demande`, `Refusé`, `Simulé`) et règles associées selon le scénario, le profil et le mode simulation.
- `docs/ROADMAP.md` §2.2 actualisé pour refléter les implémentations terminées (`🧪`/`✅`) et les choix d’architecture.

## 2026-11-13, point « Vision et capacités attendues (§2.1) »

- `analyze_document` 🧪 (`contracts/v1/aiwin11.proto`, `services/windows/DocumentReader.cs`, `local_files.rs`, `folders.rs`, `agent.py`, `core`, UI) : extraction en lecture seule du texte de documents bureautiques `.docx`, `.xlsx`, `.pptx` et `.odt` dans les dossiers autorisés (hors fichiers protégés, max 2 Mio / 16 000 caractères).
- Mode vocal continu (temps réel) 🧪 (`Speech.tsx`, `main.tsx`) : boucle conversationnelle locale enchaînant dictée SAPI, réponse en streaming et synthèse vocale locale jusqu’à arrêt.
- `set_system_setting` 🧪 (`contracts/v1/aiwin11.proto`, `SystemSettingsWriter.cs`, `Program.cs`, `agent.py`, `core`, UI) : modification utilisateur courant (HKCU) du thème clair/sombre et de la transparence Windows (`Risk::Critical`, toujours demandé, réversible).
- `admin_restore_point` 🧪 (`contracts/v1/aiwin11.proto`, `services/admin/Program.cs`, `admin.rs`, `agent.py`, `core`, UI) : création d’un point de restauration Windows via `SRSetRestorePointW` (`srclient.dll`) dans l’assistant administrateur UAC.
- Formats fichiers étendus 🧪 (`local_files.rs`) : ajout de `.json`, `.log`, `.yaml`, `.yml`, `.xml`, `.ini`, `.toml` en plus de `.txt`, `.md`, `.csv`.
- Éditeur visuel de workflows 🧪 (`Routines.tsx`) : graphe SVG interactif en nœuds et liaisons pour visualiser les étapes, conditions, reprises et branchements.
- Assistant « PC lent » 🧪 (`ControlCenter.tsx`, `main.tsx`) : bouton dédié préparant l’orchestration guidée (diagnostic lecture seule, propositions, actions approuvées, vérification et rapport).
- `send_key` 🧪 (`contracts/v1/aiwin11.proto`, `services/windows/KeyInput.cs`, `Program.cs`, `agent.py`, `core`, UI) : envoi ciblé d’une touche ou d’un raccourci clavier autorisé (liste fermée de 14 touches/raccourcis) à une fenêtre listée et non protégée via `SetForegroundWindow` + Win32 `SendInput`, classé `Risk::Critical` (toujours demandé, audit complet).
- `AgentModels.planning` 🧪 (`services/core/src/lib.rs`, `task.rs`, `Routing.tsx`) : modèle Ollama dédié à la planification (utilisé par les tâches planifiées) et tableau des modalités complété dans `Routing.tsx` (voix, texte, image, vidéo, vision, développement, planification).
- `write_text_file` 🧪 (`contracts/v1/aiwin11.proto`, `local_files.rs`, `folders.rs`, `agent.py`, `core`, UI) : création atomique d’un nouveau fichier `.txt`, `.md` ou `.csv` (64 Kio UTF-8 max) dans un dossier autorisé, sans jamais écraser un fichier existant, refusé sur les fichiers protégés.
- `list_peripherals` 🧪 (`contracts/v1/aiwin11.proto`, `services/windows/Peripherals.cs`, `agent.py`, `core`, UI) : lecture seule des périphériques Windows connectés (audio, imprimantes, caméras, écrans, Bluetooth, saisie) via `Win32_PnPEntity`.
- `open_url` 🧪 (`services/windows/BrowserLauncher.cs`, `agent.py`, `core`, UI) : ouverture d’une URL publique `https://` validée (sans identifiants, ni hôte local/loopback) dans le navigateur par défaut, toujours sur accord explicite (`Risk::Critical`).
- `docs/ROADMAP.md` §2.1 mis à jour pour refléter `list_peripherals`, `open_url`, `list_applications`/`stop_process`, LM Studio/Anthropic/Google, le routage automatique local et les actions correctives du scénario « PC lent ».

## 2026-11-13, point « Distribution (installateur, mises à jour) » (R9)

- `scripts/package.ps1` : publication .NET 10 autonome (`--self-contained true -p:PublishSingleFile=true -p:IncludeNativeLibrariesForSelfExtract=true`) par défaut pour `AiWin11.Windows.exe` et `AiWin11.Admin.exe` (option `-FrameworkDependentDotnet` conservée) ; ajout de `-BundlePython` pour télécharger et vérifier via SPDX l’archive officielle Python 3.13 embeddable dans `.tools/` ; résumé clair des composants embarqués.
- `apps/desktop/src-tauri/src/main.rs` : extraction et tests unitaires Rust de `updates_configured_with_key` et `is_allowed_update_url`.
- `apps/desktop/src/components.test.tsx` : tests unitaires Vitest pour `Updates.tsx` (canal non configuré, recherche de mise à jour, blocage pendant une tâche active).
- `docs/ROADMAP.md` et `README.md` mis à jour : ligne *Distribution (installateur, mises à jour)* passée en 🧪.

## 2026-11-13, point « Mémoire, MCP, voix, image, vidéo, workflows, multi-agents » (lots D et E)

Travail de Codex non commité enregistré tel quel (`9747305`, formatage corrigé pour que `verify.ps1` passe). Puis un sous-point à la fois, en commençant par Mémoire.

- `6828e85` Index vectoriel (migration 010, `memory_vectors`) : un vecteur chiffré DPAPI par entrée et modèle local, supprimé avec l’entrée ou à la réindexation d’un fichier ; seuls la requête et les nouvelles entrées sont recalculées. `search_memory` cite aussi jusqu’à 3 extraits de fichiers indexés, présentés comme données non fiables.
- `552991e` Mémoire de session : les 4 derniers échanges, gardés en mémoire vive par l’hôte (jamais enregistrés, jamais fournis par l’interface), aucun pour les tâches planifiées et les sous-agents, et seuls les échanges déjà traités dans le cloud partent vers un modèle cloud. Bouton **Nouvelle conversation**.
- Conservation : durée au choix (30 jours, 90 jours, 1 an, ou jusqu’à effacement) pour les notes et les rapports, appliquée au démarrage et au changement ; demandes enregistrées et index jamais effacés automatiquement.
- Non testé en réel : Ollama n’était pas démarré (pas de modèle d’embeddings). Ligne de test ajoutée à la liste.
- Multi-agents : agents personnalisés définis en données (`services/core/src/agents.rs`, réglage `agents.custom`, 8 au plus) : nom, agent de base (jamais Général), rôle ajouté aux instructions, sous-ensemble d’outils appliqué à l’exécution par le moteur de politiques (règle `agent_tools`), modèle Ollama, limite d’étapes. Utilisables directement (sélecteur d’agent) ou par délégation de l’agent Général (`perso:<nom>`), sans accord, autorisation ni contexte transmis. Éditeur dans la page Modèles.
- Ligne « Mémoire, MCP, voix, image, vidéo, workflows, multi-agents » passée en 🧪 : tout est codé et testé automatiquement, rien n’a pu être essayé en réel (Ollama arrêté, aucun moteur d’images ou de vidéo, aucun serveur MCP).
- Workflows : balises `[réessayer]` (une nouvelle tentative seulement si toutes les actions ont échoué sans refus, donc sans effet à répéter) et `[si contient:…]` (étape sautée si la réponse de la dernière étape exécutée ne contient pas le texte), éditables dans l’éditeur ; déclencheur **Quand le dossier change** (empreinte des noms, tailles et dates du dossier principal, vérifiée toutes les 30 s, l’état au moment du réglage sert de référence). Les workflows restent chiffrés avec les demandes enregistrées : pas de table `workflows` séparée.
- Vidéo : workflow ComfyUI enregistré pour l’agent (vérifié comme un envoi manuel), outil `generate_video` (toujours demandé, quota, un seul travail suivi) dont le suivi est repris automatiquement par la page Vidéo, limite quotidienne réglable (1 à 20), **Annuler la génération** (retire de la file ou interrompt seulement ce travail). Non testé en réel : ComfyUI absent.
- Image : argument `format` de `generate_image` (carré, paysage, portrait) pour les deux moteurs, outil `describe_image` (image générée par AiWin11 seulement, modèle de vision local, résultat présenté comme donnée non fiable), galerie des 24 dernières images sur la page Modèles. Aucun moteur d’images installé : non testé en réel.
- Voix : **Demande vocale** (dicter, envoyer tel quel, lire la réponse avec une voix locale), raccourci global Ctrl+Alt+V qui amène AiWin11 au premier plan et écoute, indicateur « Micro actif » dans la fenêtre et l’info-bulle de la zone de notification pendant l’écoute. La reconnaissance reste SAPI hors ligne, déclenchée seulement par l’utilisateur.
- MCP : client bivalent selon la spécification 2026-07-28 (sonde `server/discover`, `_meta` par requête, `resultType`, erreur -32022), repli sur `initialize` après une erreur ou 5 s de silence. Empreinte (taille et date) de l’exécutable et des fichiers passés en argument : un serveur mis à jour est refusé (`MCP_SERVER_CHANGED`) jusqu’à **Accepter la nouvelle version** ; enregistrer la liste ne l’accepte jamais. Bouton **Tester**. Testé avec un faux serveur en mémoire ; aucun vrai serveur MCP sur la machine.

## 2026-11-13, point « Contrôle Windows par l’agent » (lot C)

Périmètre fixé avec l’utilisateur : terminer la ligne, y compris arrêt de processus approuvé, souris/clavier en dernier recours et PowerShell depuis une liste fermée. Avancement, un élément par commit :

- 🧪 `find_control` : recherche de contrôles par nom ou type dans la fenêtre déjà lue (FindElement), sans nouvelle lecture de l’écran.
- 🧪 Redimensionnement des fenêtres (`arrange_window` : moitié gauche, moitié droite, centrée) calculé sur l’écran de la fenêtre ; test réel du pont passé sur une Table des caractères ouverte et fermée par le test.
- 🧪 `inspect_process` : détail en lecture seule d’un processus de la dernière liste (exécutable, éditeur, parent, démarrage, mémoire, threads), jamais la ligne de commande ni les titres de fenêtres.
- 🧪 `describe_screen` : capture de l’écran principal décrite par le modèle de vision local, toujours sur accord, refusée si une fenêtre protégée est visible. Pas de test réel automatique (il capturerait l’écran de l’utilisateur).
- 🧪 Plusieurs dossiers : jusqu’à quatre dossiers supplémentaires (page Fichiers), noms `alias/nom.txt` pour l’agent, jamais de copie ou de renommage entre dossiers.
- 🧪 `list_applications` : état de la liste fermée d’applications (installée, interdite, processus et fenêtres ouverts) ; test réel du pont passé.
- 🧪 `stop_process` : arrêt approuvé d’un processus de la dernière liste ; fermeture demandée sans jamais forcer s’il a une fenêtre ; processus de Windows, de sécurité et d’AiWin11 refusés au cœur, au broker et au pont. Test réel du pont passé sur des processus lancés par le test.

## 2026-11-13, statut « test utilisateur à faire »

Nouveau statut 🧪 dans `docs/ROADMAP.md` : terminé, tests automatiques au vert, mais jamais essayé dans l’application lancée. ✅ veut désormais dire terminé **et** validé (essai de l’utilisateur ou test réel sur la machine de référence). Toute nouvelle fonction entre en 🧪 ; la liste « Tests utilisateur à faire » en tête de la feuille de route sert de séance de test. Décompte : 53 ✅, 21 🧪, 90 🟡, 16 ❌ sur 180 lignes.

## 2026-11-13, reprise après Codex

**Fait** : `67966a8` travail laissé non commité par Codex, vérifié puis commité tel quel (fichiers recherche/renommage/corbeille/restauration, écrans et capture d’écran, transcription, centre de contrôle, sonde et routage adaptatif local, import/export des modèles, suivi des jetons cloud, vidéo ComfyUI, workflows à étapes, mises à jour signées). Feuille de route remise d’accord avec le code (18 lignes périmées corrigées). Nouvel outil d’agent `search_text_files` : recherche dans le nom et le contenu des fichiers du dossier de travail (lecture seule, action de politique `searchFiles`, autorisée dans tous les profils, modèles locaux seulement).

**Vérifié** : `verify.ps1` complet (12 tests Python, 102 tests Rust, linters, builds). Recherche non testée avec un modèle réel.

- Choix du modèle local en quatre stratégies (`manual`, `adaptive`, `quality_first`, `cost_optimized`), remplaçant l’interrupteur adaptatif (repris automatiquement) ; vérifié par `verify.ps1` (106 tests Rust), non testé avec Ollama réel.

- Pause et reprise d’une demande (état `paused`, au prochain point sûr, temps de pause exclu de la limite, arrêt après 30 minutes) ; étape Recover : un nouvel essai unique des lectures système après un échec passager du pont (état `recovering`). Vérifié par `verify.ps1` (109 tests Rust), non testé à la main.

- Manifestes de modèles persistés (migration 007, table `models`) : résultat du test d’appel d’outils et vision par modèle local, tableau dans **Modèles** ; les modèles en échec sont exclus du routage. Vérifié par `verify.ps1`, non testé avec Ollama réel.

- Manifestes complétés (migration 008) : fenêtre de contexte, paramètres, quantification et taille sur disque, lus dans `/api/show` et `/api/tags` ; libellés mal formés ignorés. Vérifié par `verify.ps1`, non testé avec Ollama réel (Ollama arrêté pendant la session).

- Débit (jetons/s, mesuré pendant le test d’appel d’outils) et VRAM utilisée ajoutés aux manifestes (migration 009) ; ligne de feuille de route terminée. Vérifié par `verify.ps1`, non testé avec Ollama réel.

- Placement des modèles chargés (`gpu`, `partial`, `cpu`, `unknown`, pourcentage sur le GPU) dans le Centre de contrôle et **Modèles**, avec un conseil en cas de repli sur le processeur. Vérifié par `verify.ps1`, non testé avec Ollama réel.

- Plafond de coût par demande cloud : tarif par million de jetons et coût maximal saisis dans **Modèles** (aucun prix intégré), vérifiés avant chaque appel (`CLOUD_COST_EXCEEDED`) ; le Centre de contrôle estime le coût sur 30 jours avec ce tarif (l’ancien tarif du navigateur est repris). Vérifié par `verify.ps1`, non testé avec une vraie clé.

- État de l’agent toujours visible dans la barre latérale (inactif, analyse, planification, attente d’approbation, exécution, pause, nouvel essai, erreur), déduit des étapes envoyées par le cœur, `role="status"`. Vérifié par `verify.ps1` (Vitest), non regardé dans l’application.

- Indicateur de transfert sous l’état de l’agent : traitement sur ce PC ou données envoyées au cloud (nouvelle étape `model.cloud.request` avant chaque appel cloud), puis rappel pour la dernière demande. Vérifié par `verify.ps1` (Vitest) ; pas de test Rust de l’étape (aucun faux fournisseur cloud), non testé avec une vraie clé.

- Modèle local de secours (T-009) dans **Modèles** : utilisé une seule fois si le modèle choisi échoue avant la première étape, après un appel d’outil de test réussi ; jamais vers le cloud. Vérifié par `verify.ps1` (réglage), bascule non testée avec Ollama réel.

- Temps jusqu’à l’interface prête (T-001) : premier rendu horodaté (`ui.ready`, ms depuis le lancement), affiché dans le Centre de contrôle. Vérifié par `verify.ps1` ; valeur réelle en release à relever avec l’application lancée.

- Paramètres Windows en lecture dans `get_system_status` (thème, transparence, langue, fuseau horaire, alimentation, batterie, économiseur), validés par le cœur. Vérifié par `verify.ps1` et le test réel du pont sur la machine de référence.

- Règles par ressource : fichiers protégés (modèles de noms, page Sécurité) invisibles pour l’agent et refusés à l’exécution (`FILE_PROTECTED`, audité) ; risque du manifeste ajouté à chaque décision journalisée. Vérifié par `verify.ps1`.

- Sauvegardes des données d’AiWin11 (Centre de contrôle) : copie cohérente `VACUUM INTO`, automatique avant chaque migration, 5 gardées ; restauration vérifiée appliquée au démarrage suivant après sauvegarde des données actuelles. Vérifié par `verify.ps1` (4 tests Rust) ; la prochaine ouverture de l’application sur la machine de référence fera la première sauvegarde avant migration.

- Applications interdites à l’agent (page Sécurité, liste fermée) : refus avant toute demande d’accord et au broker à l’exécution (`resource_blocked`, audité). 🧪 Vérifié par `verify.ps1`.

- Fenêtres protégées (titre ou application, page Sécurité) : absentes de la liste des fenêtres, refusées avant le pont pour la lecture, la capture, l’organisation et les clics (`WINDOW_PROTECTED`, audité). 🧪 Vérifié par `verify.ps1`.

**Prochaine étape** : test E2E de l’application, essais réels encore ouverts (UAC de l’assistant élevé, OpenAI/DeepSeek, vidéo, lecteur d’écran, recherche de fichiers et stratégies avec un vrai modèle).

## 2026-11-13 — dépôt GitHub

Le dépôt est publié sur GitHub (`https://github.com/quassim/aiwin11`, branche `main`, à la demande de l’utilisateur). Avant l’envoi : aucun secret dans les fichiers suivis, `.tools`, `.venv`, `target` et `node_modules` exclus. Pousser depuis le terminal de l’utilisateur (connexion GitHub par le navigateur) : `git push`.

## 2026-11-13, matin

**Fait** : `3dcf9a9` ADR-011 (Protobuf sur stdio au lieu de gRPC pour l’instant, R4) ; `b5f16fb` tests de composants React (Vitest + Testing Library) dans `verify.ps1` ; `4da7894` signature de code optionnelle (`AIWIN11_SIGN_THUMBPRINT`). Installateur reconstruit avec succès (assistant en fichier unique, SHA-256 épinglé).

**Vérifié** : `verify.ps1 -Live` complet sur la machine de référence : tout passe, **17/17 tests réels** avec le catalogue complet (les 2 tests qui ouvrent le Bloc-notes sautés : un Bloc-notes de l’utilisateur était ouvert, non touché).

- `31530ee` délégation d’un niveau : l’agent Général confie une sous-demande à un agent spécialisé (actions approuvées une à une, pas de re-délégation). Build release démarré : fenêtre en 2,2 s, 196 Mo privés.

**Reste** : éditeur visuel de workflows (R8), génération vidéo (R7, aucun moteur local), mises à jour automatiques (R9), test E2E de l’application complète, UAC réel de l’assistant élevé, OpenAI/DeepSeek réels, lecteur d’écran.

## 2026-11-12, nuit (suite)

**Fait** : `419bc77` mode flottant (R5) ; `a1760c3` agents spécialisés limités à leurs outils (règle `agent_scope`) ; `f3a9d82` génération d’images via un moteur Stable Diffusion local (127.0.0.1 seulement ; non testé, aucun moteur installé) ; `b860265` le verrou du cœur se rétablit après une panique.

**Vérifié** : `verify.ps1` (95 tests Rust).

**Attention** : le catalogue compte maintenant une trentaine d’outils ; avec un petit modèle local (8B), préférer un agent spécialisé pour réduire la liste.

**Reste** : R4 (processus séparés gRPC), délégation entre agents et éditeur visuel de workflows (R8), vidéo (R7), signature du code et mises à jour (R9), tests E2E de l’interface.

## 2026-11-12, nuit — R8 et MCP

**Fait** : `0f891a3` + `309223f` demandes planifiées chaque jour (modèle local, profil Observation forcé, toute demande d’accord refusée — règle `unattended`, résultats chiffrés) ; `fa69d19` client MCP stdio : page **Extensions** (5 serveurs déclarés, exécutable et arguments fixes), outils `mcp_list_tools` et `mcp_call` (toujours demandé, arguments affichés) via le broker.

**Vérifié** : `verify.ps1` (93 tests Rust). MCP et planification non testés en réel (aucun serveur MCP installé).

**Leçon** : ne jamais écrire de barre oblique inverse dans un heredoc bash contenant du Python (le Bash tool les réinterprète) : passer par un fichier écrit avec l’outil Write.

**Reste** : R4 processus séparés (gRPC), R5 overlay flottant, R7 génération d’images/vidéo (aucun moteur local installé), R8 multi-agents et éditeur visuel, R9 signature du code et mises à jour, tests E2E de l’interface.

## 2026-11-12, soir — suite autonome

**Fait** : `cf2dd43` recherche sémantique dans la mémoire (embeddings locaux Ollama, repli mots-clés) ; `233f33a` lecture à voix haute avec les voix Windows locales seulement ; `57fab10` cartes graphiques détectées par WMI (T-014) ; `5f4835b` diagnostics approuvés en liste fermée (`ipconfig /all`, `netsh wlan show interfaces`, `w32tm /query /status`) ; `aeabf8f` dossier de travail choisi dans le profil (appliqué au démarrage).

**Vérifié** : `verify.ps1` (88 tests Rust) ; état système avec GPU testé en réel. Le reste non testé en réel.

- `ca7a4ee` `organize_files` : plan de renommages prévisualisé, approuvé en une fois ; `fa36792` 16 étapes et 15 minutes max par demande ; `bd9af7b` dictée hors ligne (SAPI, moteur fr-FR présent sur la machine de référence) ; `5320b57` assistant élevé publié en fichier unique, SHA-256 épinglé et fichier verrouillé pendant l’opération.

**Prochaine étape (mise à jour)** : workflows planifiés (R8), client MCP (R6), R4 processus séparés, signature de code (R9). Ancienne liste : plan prévisualisé et approbation groupée (« Range ces fichiers ») ; reconnaissance vocale locale (Windows.Media.SpeechRecognition via le pont) ; workflows planifiés ; MCP ; R4 ; R9 (assistant élevé dans Program Files, signature).

## 2026-11-12, R6 — mémoire

**Fait** : mémoire durable chiffrée (DPAPI) : outils `remember` / `recall` via le broker, page **Mémoire**, migration 004. Vérifié par `verify.ps1` (87 tests Rust, dont le contrôle qu’aucun texte en clair n’est dans le fichier SQLite). Non testé avec un modèle réel.

- `bffc8b6` Demandes enregistrées (chiffrées, migration 005), rejouées seulement par l’utilisateur.

**Prochaine étape** : recherche sémantique (embeddings locaux Ollama) sur la mémoire et un dossier choisi ; client MCP ; R7 voix ; R8 workflows ; R4.

## 2026-11-12, DeepSeek et distribution

**Fait** : `7db6cc0` DeepSeek comme fournisseur cloud au choix (compatible OpenAI, clé séparée `AiWin11/DeepSeek`) ; `9d57c50` installateur NSIS par utilisateur (`Installateur.cmd`, `scripts/package.ps1`) embarquant pont, assistant élevé et agent, Python détecté automatiquement. L’installateur a été construit avec succès ; non installé ni testé.

**Point de sécurité ouvert** : installé par utilisateur, `AiWin11.Admin.exe` est dans un dossier modifiable par l’utilisateur ; prévoir une installation par machine de l’assistant et la signature du code (R9).

**Prochaine étape** : R6 mémoire (chiffrée DPAPI, §6) et MCP ; R7 voix locale ; R8 workflows ; R4 processus séparés.

## 2026-11-12, R3 — modèles par tâche

**Fait** : `28ee87e` vue « Modèles par tâche » (texte/actions, vision ; voix/image/vidéo signalées indisponibles), routage `manual` / `local_first` avec secours OpenAI explicite (avant toute action, accord de la demande, jamais en mode privé), modèles chargés et VRAM (`/api/ps`) ; budget en jetons par demande cloud (`CLOUD_BUDGET_EXCEEDED`, événements `model.usage`).

**Vérifié** : `verify.ps1` (85 tests Rust). Non testé en réel (pas de clé OpenAI).

**Reste en R3** : coût en euros (prix saisis par l’utilisateur), stratégies `quality_first`/`cost_optimized`/`adaptive`, tests de capacités par modèle dans l’interface, détection GPU (NVIDIA/AMD) hors Ollama, modèles d’embedding. Ensuite R4–R9.

## 2026-11-12, après-midi — suite

**Fait** : `1273304` lancement en un double-clic (`Ollama.cmd`, `AiWin11.cmd`), testé par l’utilisateur ; `cc9251c` `click_control` (UI Automation Invoke/Toggle/Select sur un contrôle numéroté de la fenêtre lue, toujours demandé) ; `4d2b4d7` Ctrl+Alt+Espace ouvre directement le champ de demande.

**Vérifié** : `verify.ps1` complet. Non testé en réel : clic UIA, palette.
- `cd572bc` `set_control_value` : remplir un champ numéroté de la fenêtre lue (SetValue, relu ; jamais un champ mot de passe ; toujours demandé). Non testé en réel.

**Prochaine étape** : PowerShell approuvé (liste fermée), SetValue UIA, dossiers autorisés ; R3 modèle par type de tâche + routage + budget ; puis R4–R9.

## 2026-11-12, midi — suite (développement seul)

**Fait** : `9a8d068` relecture complète de la conversation de conception, ROADMAP §2.5 ; `7bfad73` mode privé (T-011), icône de zone de notification (ouvrir, tout arrêter, quitter), raccourci global Ctrl+Alt+Espace, et **correction** : toutes les commandes Tauri ajoutées depuis R1 sont maintenant déclarées (`build.rs`, `capabilities/`) — avant, l’interface les aurait vues refusées.

**Vérifié** : `verify.ps1` (83 tests Rust, linters, builds). Non testé à la main.

**Prochaine étape** (ordre conseillé) : R2 UIA Invoke/SetValue sur la fenêtre inspectée (nœuds numérotés), PowerShell approuvé (liste fermée), dossiers autorisés ; R3 modèle par type de tâche + routage `local_first`/`manual` + budget ; R5 palette de commandes ; R6 mémoire/MCP ; R7 voix (STT/TTS locaux) ; R8 workflows ; R9 installateur.

## 2026-11-12, fin de matinée — avec l’utilisateur (développement seul, tests E2E par l’utilisateur)

**Fait**
- `18012c4` Assistant élevé (option B, liste validée par l’utilisateur) : `services/admin` → `AiWin11.Admin.exe`, lancé via UAC (`runas`) après accord + broker ; services (start/stop/restart, démarrage auto/manuel/désactivé) et nettoyage de `C:\Windows\Temp` (> 1 jour) ; services protégés refusés deux fois ; désactivé par défaut (**Sécurité → Opérations administrateur**). Capacités élargies à `u32`.
- `2a26a1c` Applications autorisées : Table des caractères et Paint en plus du Bloc-notes.

**Vérifié** : builds, ruff/mypy, ESLint/Prettier, fmt, 82 tests Rust, clippy. **Non vérifié** : UAC réel, lancement réel de Paint/Table des caractères, interface (l’utilisateur teste). Publier l’assistant à côté de l’exécutable (`dotnet publish services/admin/...`, voir README).

**Prochaine étape** : politiques par ressource (dossiers autorisés au-delà du dossier dédié), PowerShell approuvé (liste fermée), plan prévisualisé « Range ces fichiers », puis R3 (hub de modèles).

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
