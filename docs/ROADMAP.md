# AiWin11 — Feuille de route

État au 12 novembre 2026, dernier commit `2a26a1c` (conversation de conception relue intégralement le 12/11, points manquants ajoutés en §2.5). Ce document recoupe **tous les points de la conversation de conception** (vision initiale, cadrage v0.1, spécification v1.0, transmission à Codex) avec ce qui est réellement implémenté, puis organise le reste en phases.

Légende : ✅ fait et vérifié · 🟡 partiel · ❌ à faire · 🔷 décision à prendre.

Sources : **[V]** message de vision initial · **[S0]** cadrage v0.1 · **[S1]** spécification v1.0 · **[H]** transmission à Codex · **[B]** `docs/BACKLOG.md` (AW-xxx).

---

## 1. Synthèse

| Domaine | État |
|---|---|
| Socle (lots A et B : AW-001 à AW-012) | ✅ Application Tauri, contrats Protobuf, broker, pont .NET, passerelle Ollama/OpenAI, agent, streaming, scénario « Ouvre le Bloc-notes » validé en réel |
| Moteur de politiques, profils d’autonomie, approbations, simulation | ✅ |
| Contrôle Windows par l’agent (lot C : AW-013 à AW-017) | 🟡 Applications (liste fermée), fenêtres (lister, organiser), processus, services et état du PC en lecture, presse-papiers, fichiers `.txt` d’un dossier dédié, lecture et vision de la fenêtre précédente, opérations administrateur via l’assistant élevé ; manquent : dossiers au choix, UIA Invoke/SetValue, souris/clavier génériques, PowerShell approuvé |
| Consolidation R1 | ✅ Réglages mémorisés, arrêt global, rétention/export du journal, linters, mesures ; 🟡 accessibilité lecteur d’écran, T-003, E2E de l’interface |
| Autorisations temporaires, manifestes d’outils | ✅ |
| Architecture multi-processus gRPC [S1 §2–3] | ❌ Le cœur tourne dans le processus Tauri |
| Hub de modèles multi-tâches et routage [V][S0 §4.2][S1 §4] | 🟡 Modèle par tâche (texte/actions, vision), stratégies `manual` et `local_first` avec secours cloud explicite, modèles chargés et VRAM ; budget en jetons par demande cloud ✅ ; voix/image/vidéo, coût en euros, autres stratégies ❌ |
| Expérience bureau (overlay, palette, raccourcis, tray) [V][S0 §6] | 🟡 Zone de notification ✅, raccourci global Ctrl+Alt+Espace ouvrant le champ de demande (palette simple) ✅ ; overlay flottant ❌ |
| Mémoire, MCP, voix, image, vidéo, workflows, multi-agents (lots D et E) | 🟡 Mémoire durable chiffrée (DPAPI) ✅ ; recherche sémantique, indexation de fichiers, MCP, voix, image, vidéo, workflows, multi-agents ❌ |
| Distribution (installateur, mises à jour) | 🟡 Installateur NSIS par utilisateur (`scripts/package.ps1`) avec pont, assistant et agent embarqués, Python détecté automatiquement ; signature, mises à jour, Python/.NET embarqués ❌ |

---

## 2. Traçabilité des points de la conversation

### 2.1 Vision et capacités attendues [V]

| Point | État | Phase |
|---|---|---|
| L’IA peut faire ce que fait l’utilisateur, avec garde-fous | 🟡 Garde-fous complets ; peu d’actions disponibles | R2 |
| Choisir un fournisseur local ou en ligne | ✅ Ollama, OpenAI | — |
| Choisir un modèle **par type de tâche** (voix, texte, image, vidéo, vision, développement, planification) | ❌ Un modèle d’agent + un modèle de vision | R3 |
| Application facilement paramétrable, UI/UX soignée | 🟡 Pages Accueil, Modèles, Sécurité, Inspecteur, Fichiers, Activité | R5 |
| Contrôle des fichiers | 🟡 Dossier dédié, `.txt` seulement | R2 |
| Contrôle des logiciels (lancer, gérer) | 🟡 Bloc-notes seulement | R2 |
| Contrôle des fenêtres | ✅ Lister, premier plan, réduire, restaurer, agrandir (jamais fermer) ; lecture de la fenêtre précédente | — |
| Paramètres système | ❌ | R2 (lecture) / R9 (modification, composant privilégié) |
| Terminal / PowerShell | ❌ | R2 |
| Navigateur | ❌ | R6 (via MCP) ou R2 |
| Périphériques | ❌ | R7+ |
| Presse-papiers | ✅ Lecture et écriture de texte, accord à chaque fois (lecture réelle testée ; écriture réelle en test opt-in `AIWIN11_TEST_CLIPBOARD=1`) | — |
| Automatisations | ❌ | R8 |
| APIs natives, puis vision + souris/clavier | 🟡 APIs natives et UIA ; pas de souris/clavier génériques | R2 |
| Providers locaux : Ollama ✅, LM Studio ❌ ; cloud : OpenAI ✅, Anthropic ❌, Google ❌ | 🟡 | R3 |
| Sélection manuelle **ou routage automatique** | 🟡 Manuelle seulement | R3 |
| Multimodal : texte ✅, voix STT/TTS ❌, image ❌, vidéo ❌, compréhension d’écran 🟡 (fenêtre), analyse de documents ❌, temps réel ❌ | 🟡 | R7 |
| Configuration indépendante par modalité | ❌ | R3 |
| Agent principal qui délègue à des agents spécialisés (dev, recherche, admin Windows, bureautique, multimédia) | ❌ | R8 |
| Permissions par agent, application, dossier et action | 🟡 Par action et profil ; pas par agent, application ni dossier | R2 |
| Simulations | ✅ Mode simulation | — |
| Confirmations des opérations sensibles | ✅ | — |
| Journal d’audit | ✅ | — |
| Arrêt d’urgence | ✅ Arrêt global « Stoppe toutes les tâches » (demandé / confirmé) | — |
| Mécanismes de récupération | 🟡 Corbeille, reprise après plantage ; pas de sauvegardes ni points de restauration | R2 |
| Tableau de bord | ❌ | R5 |
| Assistant flottant | ❌ | R5 |
| Raccourcis globaux | ❌ | R5 |
| Gestion des modèles | 🟡 Détection et sélection | R3 |
| Profils d’autonomie (Observation, Assisté, Autonome contrôlé, Avancé) | ✅ | — |
| Visualisation des actions en temps réel | 🟡 Étapes, brouillon, rapport d’actions | R5 |
| Éditeur visuel de workflows | ❌ | R8 |
| Écran de routage : modèle de secours, **mode privé**, copier la configuration | ❌ | R3 |
| Détection des modèles installés ✅, capacités 🟡, conso GPU/RAM/VRAM ❌, coûts API ❌, tests de performance ❌, modèles de secours ❌, restrictions de confidentialité 🟡 | 🟡 | R3 |
| Scénario « PC lent » : examiner processus, mémoire, disque, services, proposer, appliquer les sûres, demander pour les autres, vérifier, rapport, historique | 🟡 Diagnostic en lecture seule ✅ (processus, mémoire, processeur, disques, services ; tests réels) ; actions correctives et rapport ❌ | R2 |
| Toujours protégés, même en Avancé : suppression massive, élévation de privilèges, sécurité Windows, transactions financières, transfert de données confidentielles | 🟡 Refus par défaut (aucun outil de ce type) ; règles explicites à écrire quand ces outils existeront | R2 |
| Mémoire : SQLite + recherche vectorielle | 🟡 SQLite ; pas de vectoriel | R6 |
| Plugins / MCP | ❌ | R6 |
| Moteur de permissions indépendant | ✅ | — |

### 2.2 Cadrage v0.1 [S0]

| Point | État | Phase |
|---|---|---|
| 5 engagements : liberté de choix, contrôle, extensibilité, autonomie, transparence | 🟡 Contrôle et transparence forts ; extensibilité limitée (pas de SDK) | R6 |
| Fonctionnement entièrement local possible | ✅ Avec Ollama | — |
| Couches : Dashboard, Chat ✅, Voice, Overlay, Settings 🟡, Workflow Editor | 🟡 | R5, R7, R8 |
| Core : Sessions ✅, Events ✅, Tasks ✅, State 🟡, Configuration 🟡, IPC 🟡 (Tauri seulement) | 🟡 | R4 |
| Context & Memory (RAG, index), Workflow Engine (triggers, jobs, scheduler) | ❌ | R6, R8 |
| Broker : Policy Engine ✅, Approvals ✅, Sandboxing 🟡, Audit ✅, Secrets ✅ | 🟡 | R4 |
| Stack : Tailwind CSS + composants accessibles | 🔷 CSS sans framework aujourd’hui ; décider du design system en R5 | R5 |
| Installateur signé, MSIX selon besoins | ✅ Décidé : installateur **classique** ; réalisation ❌ | R9 |
| M01 Hub : découverte ✅, config par tâche ❌, profils de génération ❌, tests de capacités 🟡 (sonde d’appel d’outil), routage auto ❌ | 🟡 | R3 |
| M02 Windows : fenêtres 🟡, applications 🟡, fichiers 🟡, processus ❌, interactions graphiques 🟡, capture d’écran 🟡, informations contextuelles 🟡 | 🟡 | R2 |
| M03 Agent : exécution ✅, planification ✅, outils ✅, délégation ❌, vérification 🟡, interruption ✅, **reprise** ❌ | 🟡 | R2, R8 |
| M04 Sécurité : règles par ressource ❌, permissions temporaires ✅, approbations ✅, journalisation ✅, arrêt immédiat ✅ | 🟡 | R2 |
| M05 Mémoire de session, préférences durables, indexation sélective, historique ✅, recherche sémantique | 🟡 | R6 |
| M06 STT/TTS, compréhension d’images 🟡, OCR, génération image/vidéo, streaming multimodal | 🟡 | R7 |
| M07 Déclencheurs, planification, conditions, tâches répétitives, reprise sur erreur, éditeur visuel | ❌ | R8 |
| M08 Installation, activation, mise à jour, permissions des plugins et serveurs MCP | ❌ | R6 |
| M09 App ✅, overlay, palette, barre système, notifications, thème, raccourcis, visualisation des agents 🟡 | 🟡 | R5 |
| M10 Journaux 🟡, suivi des coûts, statistiques modèles, traces d’agents 🟡, ressources, diagnostics | 🟡 | R9 |
| Ordre d’exécution : API native → API appli → UIA → vision + souris/clavier → script contrôlé ; **changer de méthode sans élargir les autorisations** | 🟡 API native et UIA seulement | R2 |
| Passerelle : modalités (dont **embeddings**), contraintes (VRAM, latence, coût, confidentialité, **quotas**), politiques de routage, détection réelle des capacités | 🟡 | R3 |
| Configuration YAML validée par schéma, générée par l’UI (execution_mode, privacy_mode, providers, routage par tâche, require_approval_for) | ❌ | R3 |
| Décision = identité + action + ressource + contexte + risque → ALLOW/DENY/ASK/SIMULATE | 🟡 Action, profil, contexte ; ressource et risque formel à ajouter | R2 |
| Simulateur de politique (lire dossier, modifier document, installer logiciel, suppression massive, envoi Internet, sécurité Windows) | 🟡 Table des décisions dans **Sécurité** ; actions limitées aux outils existants | R2 |
| Isolation : processus limités ✅ (agent), environnements contrôlés / VM ❌, pas d’exécution admin ✅ | 🟡 | R4, R9 |
| Approbation forte : fichiers, commande, conséquences, étendue | ✅ Pour les actions existantes | — |
| Anti-injection : contenu = données | ✅ Fichiers et fenêtres ; pages web ❌ | R6 |
| Secrets jamais donnés aux modèles | ✅ | — |
| Arrêt d’urgence global, distinguer arrêt demandé / confirmé | ✅ `stop_all_tasks` : arrêté, demandé puis confirmé | — |
| Récupération : points de restauration, sauvegardes ciblées, opérations réversibles 🟡 (corbeille), transactions compensatoires | 🟡 | R2 |
| Surfaces : Control Center 🟡, Command Palette ❌, Voice Assistant (écoute explicite, indicateur) ❌, Workflow Studio ❌ | 🟡 | R5, R7, R8 |
| UX : mode simple / mode expert | ❌ | R5 |
| UX : états d’agent toujours visibles (inactif, analyse, planification, attente, exécution, erreur) | 🟡 Pendant la demande seulement | R5 |
| UX : local / cloud / transfert de données visibles | 🟡 Badge du fournisseur ; pas d’indicateur de transfert | R5 |
| UX : changements sans redémarrage | ✅ | — |
| UX : clavier, lecteurs d’écran, résolutions | 🟡 Pas d’audit lecteur d’écran | R1 |
| Structure monorepo : `apps/overlay`, `packages/ui`, `provider-sdk`, `tool-sdk`, `plugin-sdk`, `plugins/`, `tests/{unit,integration,security,e2e}`, `scripts/` | 🟡 `packages/` et `tests/` vides ; services regroupés dans `services/core` | R4, R6 |
| Contrat d’outil : id, version, capacités, schéma, **risque, dry-run, réversible, timeout** | ✅ `manifest.rs` : un manifeste par outil, vérifié contre le catalogue et l’agent ; les invariants de politique en dérivent ; affiché dans **Sécurité** | — |
| Base MCP spec 2026-07-28 | ❌ | R6 |
| Critères NFR et scénarios | Voir §3 et §4 | — |
| Roadmap phases 0 à 5 | Voir §5 | — |
| Licence : open source, privée ou hybride | ✅ Sans objet (usage personnel) | — |
| Service privilégié facultatif | ✅ Assistant élevé à la demande (option B) : `AiWin11.Admin.exe`, liste fermée (services, fichiers temporaires), désactivé par défaut, accord + UAC ; 🟡 UAC réel non testé | — |

### 2.3 Spécification v1.0 [S1]

| Point | État | Phase |
|---|---|---|
| 6 processus (desktop, core, broker, windows, agent, model) | 🟡 desktop+core+broker+model dans Tauri ; windows et agent séparés | R4 |
| Admin futur via composant dédié limité en durée et périmètre | ✅ Assistant élevé à la demande (option B), une opération par lancement | — |
| Frontend sans terminal ni système de fichiers direct, commandes Tauri restreintes | ✅ Capacités Tauri par commande | — |
| Transport : Tauri IPC ✅, **gRPC sur named pipes avec ACL et identité** ❌, Protobuf ✅, flux d’événements ✅, HTTP local pour runtimes ✅ ; rien exposé au LAN ✅ | 🟡 | R4 |
| Enveloppe avec `payload`, identité attribuée par une couche de confiance | 🟡 Opérations typées ; identité fixée par l’hôte | R4 |
| Réponse normalisée (statuts, erreur code/message/retryable, durée) | ✅ Contrat v1 | — |
| Événements : `event_id`, `sequence`, `timestamp`, session, tâche, type, source ✅ ; **payload** ❌ (codes seulement) | 🟡 | R1 |
| Types `task.started`, `task.progress`, `model.provider.unavailable` | ✅ | — |
| Ordre par tâche ✅ ; tolérance aux doublons après reconnexion | 🟡 | R4 |
| Model Gateway : ProviderRegistry, ModelCatalog 🟡, ModelRouter (Resolve, Rank, Select, **ExplainSelection**), InferenceService (Generate, Stream ✅, Cancel 🟡), ResourceMonitor, UsageService | 🟡 | R3 |
| Manifeste de modèle (modalités, capacités, matériel, limites, statut `unverified`) | ❌ | R3 |
| Stratégies : manual ✅, local_only, cloud_only, local_first, quality_first, cost_optimized, adaptive ; élimination puis classement | ❌ | R3 |
| Aucun repli local → cloud en mode privé strict | ✅ Aucun repli n’existe ; à garantir quand le repli arrivera | R3 |
| HAL NVIDIA/AMD : GPU, pilotes, VRAM, backend, modèles chargés, repli CPU, diagnostic d’incompatibilité | ❌ | R3 |
| Agent défini en YAML (rôle, profil de modèle, capacités, limites, vérification, `retry_limit`, profil de politique) | ❌ Limites fixes dans le code | R8 |
| États : CREATED ✅, PLANNING ✅, RUNNING ✅ (`dispatching`), AWAITING_APPROVAL ✅, COMPLETED ✅, FAILED ✅, CANCELLED ✅, **PAUSED**, **CANCELLING**, **RECOVERING** ❌ (`interrupted` à la place) | 🟡 | R2 |
| Cycle Understand → Plan → Authorize ✅ → Execute ✅ → Observe ✅ → **Verify** 🟡 → **Recover** ❌ → Report ✅ | 🟡 | R2 |
| Contrat d’exécution d’outil avec `dry_run`, `timeout`, `idempotency_key` ; retour avec décision, effets, observations, statut de vérification | 🟡 Interne ; à formaliser | R2 |
| Broker : EvaluateAction ✅, RequestApproval ✅, GrantCapability / RevokeCapability temporaires ✅ (table `grants`), ExecuteAuthorized ✅, CancelExecution 🟡, GetAuditTrail ✅ (export) | 🟡 | R2 |
| Politique JSON (sujet, actions, **ressources/chemins**, contraintes `max_files`, `require_backup`, `expires_after`) | ❌ | R2 |
| Chemins canoniques, jonctions, liens | ✅ Fichiers | — |
| Invariants : action inconnue refusée ✅, pas d’auto-attribution ✅, confirmation précise ✅, pas de transmission entre agents (N/A), LLM jamais autorité ✅, erreur → refus ✅, contournement = défaillance ✅, révocation effective ✅ (autorisations relues à chaque évaluation), vérification juste avant exécution ✅ | 🟡 | R2 |
| Windows Bridge — WindowService (List ✅, Focus ✅, Minimize/Restore/Maximize ✅, Resize ❌, Close ❌ volontairement absent) | 🟡 | R2 |
| ApplicationService (Launch ✅ liste fermée : Bloc-notes, Table des caractères, Paint ; Discover ❌, GetStatus ❌) | 🟡 | R2 |
| FileService (List ✅, Read ✅, Copy ✅, Move ✅, Delete 🟡 corbeille, **Search** ❌) | 🟡 | R2 |
| ProcessService (List ✅ lecture seule, Inspect ❌, Terminate ❌) | 🟡 | R2 |
| DesktopService (CaptureScreen, GetDisplays, GetActiveWindow) | 🟡 Capture d’une fenêtre | R2 |
| UIAutomationService (InspectTree ✅, Invoke ✅ et SetValue ✅ sur un contrôle numéroté de la fenêtre lue, FindElement ❌) | 🟡 | R2 |
| InputService (MovePointer, Click, Type 🟡, Hotkey) | 🟡 | R2 |
| ShellService (ExecuteApprovedCommand) | ❌ | R2 |
| SystemService (GetSystemMetrics ✅ mémoire, processeur, disques, durée de fonctionnement ; GetHardwareInfo ❌) | 🟡 | R2, R3 |
| DPI, multi-écran, défilement, focus, références UIA périmées, dialogues modaux | ❌ Non testé | R2 |
| Capture jamais envoyée au cloud sans politique | ✅ Vision locale seulement | — |
| Services : Memory, Workflow, Speech, Image, Video, Plugin, Telemetry (local) | ❌ | R6–R9 |
| MCP avec passerelle de sécurité ; autorisation MCP ≠ permission AiWin11 | ❌ | R6 |
| Tables : sessions ✅, tasks ✅, tool_executions ✅, events ✅, settings ✅ ; providers, models, model_profiles, agents, policies, approvals, memory_entries, workflows, artifacts ❌ | 🟡 | R2, R3, R6, R8 |
| Dates UTC ✅, migrations versionnées ✅, clés hors SQLite ✅, chiffrement : pas de chiffrement de la base, rien de sensible stocké (✅ décidé, §6), durée de conservation et effacement ✅ | ✅ | — |
| Profil IA : stratégie, capacités requises, `allow_cloud_fallback`, `allow_screen_upload`, `allow_document_upload`, **budget par tâche** | ❌ | R3 |
| Lots A ✅, B ✅, C 🟡, D ❌, E ❌ | — | — |
| Matrice de tests T-001 à T-014 | Voir §3 | — |
| Exigences de performance | Voir §4 | — |
| Risques : UIA (fallback visuel) ❌, GPU ❌, actions incorrectes ✅, injection ✅, suppression ✅ (corbeille), fuite cloud ✅, fragmentation providers 🟡, conso GPU ❌, dépendance frameworks 🟡 | 🟡 | R2, R3 |
| Signaler les fenêtres élevées ou protégées au lieu de contourner | 🟡 Affinité d’affichage refusée ; élévation non détectée explicitement | R2 |
| À valider par prototype : perf Rust/.NET/Python, IPC définitif, stabilité UIA, capture, packaging, empreinte mémoire | ❌ | R1, R4, R9 |
| À adapter au matériel : taille des modèles, accélération, modèles simultanés, quantification, latence, VRAM | ❌ (RTX 3090 utilisée pour les tests) | R3 |

### 2.4 Transmission [H] et règles du dépôt (AGENTS.md)

| Point | État | Phase |
|---|---|---|
| Dépôt privé pour versionner le travail de Codex | ✅ Décidé : dépôt git **local** uniquement | — |
| CI sur un vrai runner Windows 11 | 🟡 Workflow présent mais pas exécuté (dépôt local) ; remplacé par `scripts/verify.ps1` sur la machine de référence | R1 |
| Code linté et typé | ✅ clippy, TypeScript strict, ESLint (règles typées) + Prettier, ruff + mypy strict, `cargo fmt --check`, tous dans `scripts/verify.ps1` ; 🟡 pas d’analyseurs .NET supplémentaires | — |
| Tests unitaires, intégration, sécurité | ✅ 82 tests Rust, 11 Python, 19 tests réels | — |
| Aucune opération hors broker | ✅ | — |
| APIs publiques et migrations documentées | ✅ `docs/CONTRACTS.md` | — |
| README : installation, lancement, dépannage | ✅ | — |
| Pas de télémétrie, pas de serveur exposé au LAN | ✅ | — |
| UI accessible, badges local/cloud, activité de l’agent | 🟡 | R1, R5 |

### 2.5 Points de la conversation ajoutés après relecture complète (12/11)

| Point | Source | État | Phase |
|---|---|---|---|
| Limites d’un agent : `max_steps` 30, `max_tool_calls` 50, `max_runtime_seconds` 900, `max_parallel_tools` 2, `retry_limit` 2, `verification_required` | [S1 §5.1] | 🟡 8 étapes max, une action à la fois, vérification par outil ; pas de durée max ni de reprise automatique | R2, R8 |
| Résilience : la panne d’un provider ou d’un agent ne fait pas tomber le noyau ; « crash d’un agent : core et interface restent fonctionnels » | [S0 §8][S1 §11] | 🟡 Agent Python et pont en processus séparés avec délais ; non testé par injection de pannes | R4, R9 |
| Plafond de coût par tâche (`max_cost_usd_per_task`), vérification du budget avant chaque appel cloud | [S1 §9] | ❌ | R3 |
| Mode privé / local strict : aucun transfert cloud, jamais de bascule local→cloud | [V][S0 §8][S1 §4.3], T-011 | ✅ Interrupteur **Mode privé** (Sécurité), aucun fallback automatique | — |
| Intégration profonde : agent permanent, interfaces contextuelles, événements Windows, automatisation proactive | [V §7] | ❌ | R5, R8 |
| Zero-trust entre agents et outils ; un agent ne transmet pas d’autorisation à un autre | [S0 §10][S1 §6.3] | ✅ pour l’agent unique (broker, identité attribuée par le cœur) ; à revalider avec le multi-agents | R8 |
| Périphériques, terminal, navigateur parmi les capacités | [V §2] | ❌ Navigateur via MCP prévu ; périphériques non planifiés en détail | R2, R6 |
| Agents spécialisés (développement, recherche, administration Windows, bureautique, multimédia) | [V §2] | ❌ | R8 |
| Isolation des tâches risquées (processus limités, environnement contrôlé, VM si pertinent) | [S0 §5] | 🟡 Processus séparés, utilisateur standard, assistant élevé une opération ; pas de bac à sable ni de VM | R4 |
| Licence (open source, privée, hybride) | [S0 §10] | 🔷 À décider (usage personnel pour l’instant) | — |
| Coûts API, consommation GPU/RAM/VRAM et tests de performance par modèle dans le gestionnaire de modèles | [V §4] | ❌ | R3, R9 |

---

## 3. Scénarios et matrice de tests [S0 §8][S1 §11]

| Réf. | Scénario / test | État |
|---|---|---|
| — | « Ouvre le Bloc-notes » | ✅ Réel |
| — | « Résume cette fenêtre » | ✅ Réel (structure et description locale) |
| — | « Range ces fichiers » : **plan prévisualisé**, puis actions contrôlées | 🟡 Actions une à une avec approbation ; pas de plan global prévisualisé ni d’approbation groupée — R2 |
| — | « Utilise mon modèle local » : aucun appel cloud | ✅ |
| — | « Stoppe toutes les tâches » | ✅ Arrêt global (tests unitaires ; réel non testé) |
| — | Provider indisponible : bascule autorisée ou erreur compréhensible | 🟡 Erreur ✅, bascule ❌ — R3 |
| — | Instruction malveillante dans une page | 🟡 Fichiers ✅ ; pages web ❌ — R6 |
| — | Redémarrage après interruption | ✅ |
| T-001 | Démarrage à froid, services démarrés et états visibles | 🟡 Fenêtre principale créée en 2,9 s à froid, 0,05 s à chaud (`scripts/measure.ps1`, 12/11/2026) ; « interface prête » pas encore instrumentée — R1 |
| T-002 | Provider local | ✅ |
| T-003 | Provider cloud | ❌ Jamais testé avec une vraie clé — R1 |
| T-004 | Changement de modèle sans redémarrage | ✅ |
| T-005 | Action Windows simple vérifiée | ✅ |
| T-006 | Action non autorisée refusée et journalisée | ✅ |
| T-007 | Aucune exécution avant approbation | ✅ |
| T-008 | Annulation | ✅ Unitaires ; réel ❌ — R1 |
| T-009 | Panne du modèle | 🟡 Erreur contrôlée ; pas de repli — R3 |
| T-010 | Injection de prompt | 🟡 Fichiers ✅ ; suite complète AW-024 — R1 |
| T-011 | Mode privé : aucun transfert réseau cloud | 🟡 Garanti par conception ; pas de test réseau ni de mode privé explicite — R3 |
| T-012 | Reprise après crash | ✅ |
| T-013 | Multi-écran / DPI | ❌ — R2 |
| T-014 | Détection GPU NVIDIA/AMD | ❌ — R3 |

## 4. Exigences non fonctionnelles [S0 §8][S1 §11]

| Exigence | État |
|---|---|
| Palette p95 < 200 ms | ❌ Pas de palette — R5 |
| Évaluation d’une permission p95 < 50 ms | ✅ p95 7,1 ms, p50 6,6 ms (politique + autorisation du broker journalisée sur SQLite disque, 500 essais, release, machine de référence, 12/11/2026) |
| Navigation p95 < 150 ms | ❌ Non mesuré — R1 |
| Interface prête < 3 s | 🟡 Fenêtre créée en 2,9 s au premier démarrage à froid ; le rendu React n’est pas encore horodaté — R1 |
| RAM au repos < 350 Mo hors modèles | ✅ 178–182 Mo de mémoire privée pour 9 processus (application, WebView2, pont de réserve) après 15 s ; ensemble de travail cumulé 358–366 Mo, qui compte plusieurs fois les pages partagées de WebView2 |
| Crash d’un agent : cœur et UI fonctionnels | ✅ L’agent Python est un processus séparé, borné dans le temps |
| 100 % des appels d’outils par le broker, 100 % des actions tracées | ✅ |
| Aucune requête cloud en mode local strict | ✅ Par conception — test réseau R3 |
| Changement de provider sans redémarrage global | ✅ |
| Machine de référence définie | ✅ PC de l’utilisateur (i9-12900K, 32 Go, RTX 3090, Windows 11 Pro) — §6 |

---

## 5. Phases

L’ordre suit le principe de la conversation : sécurité et fiabilité avant l’autonomie avancée, une version testable à chaque étape. Les phases R1 à R3 terminent la « Phase 1–2 » de la roadmap initiale [S0 §9] ; R4 à R9 couvrent les phases 3 à 5.

### R1 — Consolider le produit (prochaine étape)

- Distribution de développement simplifiée : embarquer l’agent Python et le pont .NET, supprimer `AIWIN11_PYTHON` manuel ; script de démarrage unique (`scripts/`).
- ✅ Réglages mémorisés : fournisseur, modèle, modèle de vision (table `settings`, événement `settings.models.changed`).
- ✅ **Arrêt global** (« Stoppe toutes les tâches ») avec état demandé / confirmé ; bouton toujours visible.
- ✅ **Durée de conservation et effacement** de l’historique ; export du journal d’audit (JSON).
- ✅ Événements manquants : `task.started`, `task.progress`, `model.provider.unavailable` ; charge utile = code non sensible (fournisseur, numéro d’étape). 🟡 Pas de payload JSON structuré.
- ✅ Qualité : ESLint + Prettier, ruff + mypy, `cargo fmt --check`. ❌ Tests de composants React, test E2E de l’application.
- 🟡 Mesures : RAM au repos ✅, évaluation de politique ✅, démarrage (fenêtre) 🟡 ; navigation et « interface prête » ❌ (`scripts/measure.ps1`, test `measure_permission_decision_latency`).
- Tests manquants : T-003 (OpenAI réel), T-008 annulation réelle, suite d’injection de prompt (début d’AW-024).
- Audit d’accessibilité au lecteur d’écran (Narrateur, NVDA).
- Script local `scripts/verify.ps1` qui remplace la CI (dépôt local) : builds, analyse statique, tests unitaires et, en option, tests réels.
- ✅ Test qui garantit l’invariant « rien de sensible dans SQLite » (voir §6) : schéma revu colonne par colonne, garde `journal_code` qui remplace tout texte libre par `REDACTED`, scan du fichier de base.

### R2 — Politiques par ressource et contrôle Windows étendu (fin du lot C, début de M02/M04)

- 🟡 Politiques en données : sujet, action, **ressource (dossiers autorisés, applications)** ❌, contraintes (`max_files`, `require_backup`, expiration) ❌, **autorisations temporaires et révocables** ✅ (table `grants`, section de **Sécurité**) ; table `policies` ❌.
- ✅ Risque formel par outil et **manifeste d’outil** (risque, dry-run, réversible, timeout, capacités).
- Règles critiques explicites (toujours demandées ou refusées) : suppression massive, élévation, sécurité Windows, transactions financières, envoi de données confidentielles, installation de logiciel.
- Pont Windows :
  - ApplicationService : découverte et lancement d’applications autorisées ;
  - 🟡 WindowService : lister ✅, focus ✅, réduire/restaurer/agrandir ✅ ; redimensionner ❌ ; fermer ❌ (risque de perte de données, à décider) ;
  - 🟡 ProcessService : lister ✅, inspecter ❌, arrêter (avec approbation) ❌ ;
  - UIAutomationService : FindElement, Invoke, SetValue ;
  - InputService : clic, saisie, raccourcis, en dernier recours ;
  - ShellService : commandes PowerShell **approuvées** depuis une liste ;
  - ✅ Presse-papiers (lecture et écriture avec approbation) ;
  - 🟡 SystemService : métriques ✅ (mémoire, processeur, disques), matériel ❌ ;
  - DesktopService : écrans, capture d’écran complète.
- ✅ **Assistant élevé à la demande** (option B, §6) pour les opérations administrateur d’une liste fermée : double validation AiWin11 + UAC, une opération par lancement, puis arrêt.
- Changement de méthode (API → UIA → vision → souris/clavier) **sans élargir les autorisations**.
- Fichiers : plusieurs dossiers autorisés, recherche, autres formats ; restauration et vidage de la corbeille ; sauvegarde avant modification (`require_backup`).
- Scénarios « Range ces fichiers » (plan prévisualisé, approbation groupée) et « PC lent » (diagnostic, actions sûres, rapport).
- États PAUSED, CANCELLING, RECOVERING ; étapes Verify et Recover, `retry_limit`.
- Tests T-013 (multi-écran, DPI), références UIA périmées, dialogues modaux, fenêtres élevées signalées.

### R3 — Hub de modèles (M01, AW-020)

- Tables `providers`, `models`, `model_profiles` ; ProviderRegistry, ModelCatalog avec manifeste (statut `unverified` jusqu’au test).
- **Modèle par tâche** : planification, contrôle Windows, développement, vision, STT, TTS, image, vidéo, embeddings.
- Stratégies de routage : manual, local_only, cloud_only, local_first, quality_first, cost_optimized, adaptive ; **ExplainSelection**.
- **Mode privé** explicite, repli autorisé ou non, jamais de repli local → cloud en mode strict (test réseau T-011).
- Providers : LM Studio, Anthropic, Google Gemini.
- HAL GPU NVIDIA/AMD : pilotes, VRAM, backend, modèles chargés, repli CPU, diagnostic (T-014).
- UsageService : coûts estimés, **budget par tâche** vérifié avant chaque appel, quotas.
- Configuration exportable/copiable, validée par schéma (YAML).
- Tests de capacités et de performance par modèle.

### R4 — Architecture multi-processus (ADR-002, ADR-007)

- Prototype puis mise en place de gRPC sur named pipes avec ACL et vérification de l’identité de chaque processus.
- Séparer `aiwin11-core`, `aiwin11-broker`, `aiwin11-model` du processus Tauri ; supervision et redémarrage.
- Déplacer la planification dans le runtime d’agent Python (ADR-005), le broker restant seul juge.
- Livraison des événements tolérante aux doublons après reconnexion ; tests de contrats.
- Bac à sable renforcé pour les tâches risquées.

### R5 — Expérience bureau (M09, AW-022)

- Overlay / assistant flottant, **palette de commandes** (raccourci global, p95 < 200 ms), icône de barre système, notifications.
- Tableau de bord (Control Center) : modèles, agents, permissions, tâches, ressources.
- Mode simple / mode expert ; **état de l’agent toujours visible** ; indicateur local / cloud / transfert de données.
- 🔷 Design system (Tailwind + composants accessibles, ou alternative) ; thème proche de Windows 11 avec une identité propre.

### R6 — Mémoire, extensions et MCP (M05, M08, AW-018, AW-019)

- Mémoire de session, préférences durables, indexation sélective de fichiers, recherche sémantique (SQLite + index vectoriel, embeddings locaux), suppression et conservation.
- SDK d’outils et de providers ; client MCP (spec 2026-07-28) derrière le broker ; gestion des serveurs MCP (installation, activation, mise à jour, permissions).
- Navigateur et pages web comme données non fiables ; tests d’injection depuis une page.

### R7 — Multimodal (M06, AW-021, AW-027, AW-028)

- Voix : STT et TTS locaux, écoute explicite avec indicateur visible.
- OCR et analyse de documents ; capture d’écran complète.
- Génération d’images multi-provider ; jobs vidéo asynchrones avec suivi et quotas.
- Contrôle des transferts : aucune image ou document vers le cloud sans politique explicite.

### R8 — Workflows et multi-agents (M07, AW-023, AW-025, AW-026, AW-029)

- Workflows simples, déclencheurs, planification, conditions, reprise sur erreur ; table `workflows`.
- Agents spécialisés définis en données (YAML) : développement, recherche, administration Windows, bureautique, multimédia ; délégation sans transmission d’autorisations.
- Éditeur visuel de workflows ; automatisations événementielles avancées.

### R9 — Observabilité et distribution (M10, AW-024, AW-030 à AW-032)

- Journaux techniques, traces d’agents, statistiques des modèles, ressources, diagnostics ; suivi des coûts avancé.
- Suite complète de tests de sécurité et de robustesse (AW-024).
- Catalogue local de plugins (AW-031).
- **Installateur classique** (NSIS ou MSI via Tauri) embarquant le pont .NET, l’assistant élevé, l’agent Python et leurs runtimes ; mises à jour ; diagnostics (AW-032). Signature de code facultative pour un usage personnel.

---

## 6. Décisions

### Prises (12 novembre 2026)

| Décision | Choix | Conséquence |
|---|---|---|
| Dépôt | **Local uniquement** pour le moment | La CI GitHub n’est pas exécutée : R1 ajoute un script local qui lance toutes les vérifications (`scripts/verify.ps1`) |
| Licence | **Sans objet** : usage personnel, un seul utilisateur | Aucune action |
| Installateur | **Classique** (pas de MSIX) | R9 : installateur Tauri NSIS ou MSI, avec le pont .NET, l’agent Python et leur runtime |
| Chiffrement SQLite | **Pas de chiffrement de la base entière pour l’instant** | Voir ci-dessous |
| Composant administrateur | **Option B : assistant élevé à la demande** (détail ci-dessous) | Réalisé en R2, avec les actions système |
| Machine de référence | **Le PC de l’utilisateur** : Intel Core i9-12900K (16 cœurs / 24 threads), 32 Go de RAM, NVIDIA RTX 3090 24 Go (pilote 621.14), SSD Samsung 980 PRO 1 To, Windows 11 Professionnel build 26300 | Toutes les mesures de performance (R1) sont faites et comparées sur cette machine |

**Chiffrement SQLite — justification.** Aujourd’hui la base ne contient aucune donnée sensible : seulement des types d’événements, des codes de décision, des états de tâche et le profil de sécurité. Les demandes, les réponses, les noms et contenus de fichiers, les titres de fenêtres et le texte saisi n’y sont jamais écrits ; la clé API est dans le Gestionnaire d’identifiants Windows. Chiffrer la base entière (SQLCipher) ajouterait une dépendance native et une gestion de clé sans rien protéger de plus. Deux règles à la place :

1. **Invariant** : rien de sensible dans SQLite tant qu’il n’existe pas de chiffrement de contenu. Tout ajout de données utilisateur à la base doit passer par la règle 2. Des tests le vérifient (`the_schema_only_holds_identifiers_codes_and_timestamps`, `free_text_never_reaches_the_database_file`) ; toute nouvelle colonne fait échouer le premier tant qu’elle n’est pas revue.
2. **Quand la mémoire arrivera (R6)** : les contenus mémorisés (textes, extraits, embeddings) seront chiffrés champ par champ avec une clé protégée par DPAPI pour l’utilisateur Windows courant ; les métadonnées non sensibles restent en clair pour les requêtes.

Recommandation hors application : activer BitLocker sur le disque système protège la base, les fichiers et la corbeille d’AiWin11 en cas de vol du PC.

### Encore ouvertes 🔷

| Décision | Contexte |
|---|---|
| Design system (Tailwind ou autre) | [S0 §3] ; CSS simple aujourd’hui ; à décider en R5 |

### Composant administrateur — option B retenue

**De quoi il s’agit.** AiWin11 tourne avec les droits d’un utilisateur normal, comme exigé par la spécification. Certaines actions Windows exigent les droits administrateur : installer ou désinstaller un logiciel, arrêter ou configurer un service, modifier des réglages système protégés (pare-feu, Defender, registre machine), nettoyer des fichiers système. Pour le scénario « PC lent », par exemple, une partie des optimisations en dépend. Trois options :

| Option | Fonctionnement | Avantages | Inconvénients |
|---|---|---|---|
| A. Jamais d’administration | L’agent prépare l’action et te l’explique ; tu la fais toi-même | Aucun risque d’élévation | L’agent ne peut pas finir certaines tâches seul |
| B. Assistant élevé à la demande | Un petit programme séparé, lancé seulement pour une action précise, avec l’invite UAC de Windows à chaque fois, puis il s’arrête | Tu valides deux fois (AiWin11 puis Windows) ; rien d’élevé ne reste actif | Développement supplémentaire ; une invite UAC à chaque action |
| C. Service Windows permanent | Un service installé tourne en permanence avec des droits élevés | Pas d’invite UAC | Surface d’attaque permanente ; contraire à l’esprit de la spécification |

**Choix : B.** Exigences de conception, en complément de la spécification ([S0 §10], [S1 §2.1]) :

- **Exécutable séparé et minimal** (`AiWin11.Elevated.exe`), sans réseau, sans interface ; il n’accepte qu’**une** opération typée v1 par lancement, depuis une liste fermée (par exemple : arrêter un service, désinstaller un programme identifié, nettoyer les fichiers temporaires système). Jamais de commande libre ni de script.
- **Double validation** : la politique AiWin11 classe ces opérations comme **critiques** (toujours demandées, dans tous les profils, refusées en Observation), puis Windows affiche l’**invite UAC** au lancement. Refuser l’une ou l’autre annule l’action.
- **Durée de vie limitée** : lancé pour l’opération, il vérifie son échéance, exécute, renvoie un résultat vérifié et se termine ; rien d’élevé ne reste actif. Pas de service installé, pas de tâche planifiée.
- **Canal privé** : la commande arrive par un canal hérité ou un named pipe à ACL limitée à l’utilisateur ; l’assistant vérifie l’identité de l’appelant et l’intégrité de la commande.
- **Audit** : décision, invite UAC acceptée ou refusée, résultat ; jamais de données sensibles.
- **Installation** : l’assistant est livré par l’installateur classique, à côté du pont, signé si possible pour que l’invite UAC affiche un éditeur identifié.
- L’option C (service permanent) reste exclue.
