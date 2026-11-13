# AiWin11 — Feuille de route

État au 13 novembre 2026, dernier commit `9a6c583` (conversation de conception relue intégralement le 12/11, points manquants ajoutés en §2.5). Ce document recoupe **tous les points de la conversation de conception** (vision initiale, cadrage v0.1, spécification v1.0, transmission à Codex) avec ce qui est réellement implémenté, puis organise le reste en phases.

Légende : ✅ terminé et validé (essayé par l’utilisateur ou par un test réel sur la machine de référence, ou sans objet à tester : décision, documentation) · 🧪 terminé, test utilisateur à faire (code complet, tests automatiques au vert, jamais essayé dans l’application lancée) · 🟡 partiel · ❌ à faire · 🔷 décision à prendre. Les sous-points d’une ligne suivent la même légende.

Sources : **[V]** message de vision initial · **[S0]** cadrage v0.1 · **[S1]** spécification v1.0 · **[H]** transmission à Codex · **[B]** `docs/BACKLOG.md` (AW-xxx).

### Avancement depuis l’état initial

- **R2**. Le bureau recherche, renomme et met à la corbeille les fichiers texte, Markdown et CSV. Il restaure aussi les éléments de la corbeille sans écrasement. L’Inspecteur liste les écrans et propose la capture locale d’un écran choisi.
- **R3 et R5**. Le bureau propose un centre de contrôle, deux modes de navigation, un test des appels d’outils Ollama, le transfert JSON des choix de modèles et le suivi des jetons cloud avec une estimation de coût. Le choix du modèle local propose quatre stratégies (manuel, adaptatif selon la VRAM, qualité d’abord, économie) et vérifie l’appel d’outil avant tout autre modèle.
- **R7**. L’Inspecteur transcrit les captures avec Ollama. La page Vidéo soumet et suit un workflow ComfyUI sur la boucle locale, avec un seul travail suivi à la fois. La génération sur un modèle vidéo réel reste à vérifier.
- **R8**. Les workflows enregistrés prennent en charge des étapes distinctes, les branches après échec, la reprise manuelle, les jours choisis et le démarrage d’AiWin11. Les exécutions planifiées restent locales et en lecture seule.
- **R9**. Le script de distribution (`scripts/package.ps1`) embarque par défaut le runtime .NET 10 autonome pour le pont et l’assistant, peut embarquer Python 3.13 (`-BundlePython` / `-PythonRuntimeZip`), signe optionnellement les binaires (`AIWIN11_SIGN_THUMBPRINT`) et produit une mise à jour NSIS signée avec son manifeste `latest.json`. La publication du canal de production sur GitHub et l’essai d’installation sur une machine vierge restent à valider en réel.

### Décompte (lignes des tableaux §1 à §4)

| État | Lignes | Part |
|---|---|---|
| ✅ Terminé et validé | 53 | 29 % |
| 🧪 Terminé, test utilisateur à faire | 30 | 17 % |
| 🟡 Partiel | 84 | 47 % |
| ❌ À faire | 13 | 7 % |

Une ligne 🟡 peut contenir des sous-points 🧪 : ils comptent dans la séance de test ci-dessous.

### Tests utilisateur à faire (lignes 🧪)

À essayer dans l’application lancée, avec Ollama démarré ; une ligne passe à ✅ une fois essayée sans problème.

- [ ] R1. Après un redémarrage de Windows, lancer `scripts/measure.ps1 -Runs 1` avant d’ouvrir AiWin11 et relever `ReadyFromStartMs`.
- [ ] R1. Avec Narrateur ou NVDA, vérifier l’annonce du titre via le lien d’évitement, de la demande d’accord et du changement d’état après **Refuser**.
- [x] ✅ Architecture gRPC. Dans la release, écrans listés dans **Inspecteur**, lancement du Bloc-notes après approbation et exécution confirmée dans le journal.
- [ ] Agent principal qui délègue à des agents spécialisés (dev, recherche, admin Windows, bureautique, multimédia)
- [x] ✅ Arrêt d’urgence pendant une approbation, depuis le bouton et via Ctrl+Alt+Retour arrière.
- [x] ✅ Tableau de bord ouvert et actualisé dans la release.
- [x] ✅ Assistant flottant activé, puis retour à la taille normale.
- [ ] Raccourcis globaux
- [ ] Écran de routage : modèle de secours, **mode privé**, copier la configuration
- [x] ✅ Plugins / MCP : déclaration, sonde, liste et appel approuvés d’un outil de test local, mise à jour bloquée puis acceptée.
- [ ] Arrêt d’urgence global, distinguer arrêt demandé / confirmé
- [x] ✅ UX : mode simple / mode expert. La visibilité de l’Inspecteur suit le changement de mode.
- [ ] UX : états d’agent toujours visibles (inactif, analyse, planification, attente, exécution, erreur)
- [ ] UX : local / cloud / transfert de données visibles
- [ ] Service privilégié facultatif
- [ ] Admin futur via composant dédié limité en durée et périmètre
- [ ] Manifeste de modèle (modalités, capacités, matériel, limites, statut `unverified`)
- [ ] Hub. Vérifier le modèle de l’agent Développement et la dictée avec un moteur SAPI choisi. Essayer LM Studio réel et les sondes Gemini et Claude avec leurs clés. Vérifier le mode Mixte avec et sans accord cloud, ainsi que la génération avec un modèle d’images choisi.
- [ ] Aucun repli local → cloud en mode privé strict
- [ ] Plafond de coût par tâche (`max_cost_usd_per_task`), vérification du budget avant chaque appel cloud
- [ ] Mode privé / local strict : aucun transfert cloud, jamais de bascule local→cloud
- [ ] « Range ces fichiers » : **plan prévisualisé**, puis actions contrôlées
- [x] ✅ « Stoppe toutes les tâches » annule la tâche en attente d’approbation sans exécuter l’action proposée.
- [ ] Lot C (AW-013 à AW-017). Inspecter et décrire une fenêtre avec le modèle local. Rechercher un contrôle, approuver un clic puis une saisie et vérifier le journal. Copier, renommer et mettre un fichier de test à la corbeille dans un dossier autorisé. Relancer AiWin11 après une tâche interrompue et vérifier son état dans l’historique.
- [ ] Mémoire (lot D). Avec un modèle d’embeddings local choisi, retenir deux informations, indexer un fichier texte, puis demander à l’agent de chercher dans sa mémoire : il cite la note et l’extrait du fichier. Poser une question de suivi (« et ensuite ? ») : l’agent tient compte de l’échange précédent ; **Nouvelle conversation** l’oublie. Choisir une durée de conservation.
  - ✅ Indexation d’un fichier, recherche par son contenu et retrait de l’extrait validés dans la release.
  - ✅ Conversation : Ollama rappelle un code synthétique à la demande suivante et l’oublie après **Nouvelle conversation**.
- [x] ✅ MCP (lot D). Sonde, liste, appel approuvé et acceptation d’une mise à jour validés avec un serveur stdio de recette. Le test Rust couvre aussi le refus d’accès de l’agent après changement du fichier.
- [ ] Voix (lot D). Depuis une autre application, appuyer sur Ctrl+Alt+V : AiWin11 passe devant, l’indicateur « Micro actif » s’affiche (fenêtre et info-bulle de la zone de notification), la demande dite est envoyée et la réponse est lue avec une voix locale.
- [ ] Image (lot D). Avec un moteur local configuré, demander « génère un paysage de montagne au format paysage », vérifier l’image dans la galerie de la page Modèles, puis demander à l’agent de la décrire avec le modèle de vision.
- [ ] Vidéo (lot D). Avec ComfyUI et un modèle vidéo : enregistrer le workflow pour l’agent, demander une vidéo à l’agent, approuver, suivre dans la page Vidéo ; essayer **Annuler la génération** et la limite quotidienne.
- [ ] Workflows (lot E). Créer un workflow à deux étapes dont la seconde a la condition « Seulement si la réponse précédente contient », cocher **Réessayer une fois** sur la première, l’exécuter ; choisir **Quand le dossier change** puis ajouter un fichier au dossier de l’agent et retrouver le rapport.
  - ✅ Exécution locale depuis l’interface : deux étapes terminées et une condition non satisfaite, avec les événements correspondants dans le journal.
  - ✅ Déclenchement par ajout d’un fichier immédiatement après activation et rapport visible dans la release. Reste l’essai de la nouvelle tentative après échec.
- [ ] Multi-agents (lot E). Page Modèles : créer un agent personnalisé (base Recherche, seul outil `read_text_file`, rôle court), le choisir pour une demande, puis demander à l’agent Général de lui déléguer une lecture ; vérifier qu’un autre outil est refusé (`agent_tools`) dans le journal.
- [x] ✅ PowerShell approuvé. Le diagnostic prédéfini de l’édition et du build Windows s’exécute après **Autoriser** et reste inexécuté après **Refuser**, selon le journal.
- [ ] Sous-points 🧪 des lignes partielles : applications interdites et fenêtres protégées (Sécurité), placement GPU, manifestes (débit, VRAM), modèle local de secours, fichiers protégés, sauvegardes et restauration, temps « interface prête », paramètres Windows dans l’état du PC

---

## 1. Synthèse

| Domaine | État |
|---|---|
| Socle (lots A et B : AW-001 à AW-012) | ✅ Application Tauri, contrats Protobuf, broker, pont .NET, passerelle Ollama/OpenAI, agent, streaming, scénario « Ouvre le Bloc-notes » validé en réel |
| Moteur de politiques, profils d’autonomie, approbations, simulation | ✅ |
| Contrôle Windows par l’agent (lot C : AW-013 à AW-017) | 🧪 Inspection et recherche UIA, capture et vision locales, clic et saisie sur un contrôle avec recours Win32 ciblé, opérations sur les fichiers des dossiers autorisés, audit et reprise. Inclut la découverte des applications, l’organisation des fenêtres, le détail et l’arrêt approuvé d’un processus et le diagnostic PowerShell sur accord. |
| Consolidation R1 | 🧪 Réglages, arrêt global, export, annulation gRPC, injections ciblées, fournisseur DeepSeek réel et parcours de refus dans la release validés. Premier rendu après redémarrage et annonces du lecteur d’écran à vérifier. |
| Autorisations temporaires, manifestes d’outils | ✅ |
| Architecture multi-processus gRPC [S1 §2–3] | 🧪 Cœur, broker, passerelle de modèles, pont Windows et agent dans des processus distincts de Tauri. Session, tâches, autorisations, modèles et journal sur gRPC local. Reprise par curseur et validation Windows automatisées. |
| Hub de modèles multi-tâches et routage [V][S0 §4.2][S1 §4] | 🧪 Routage local, cloud et mixte, registre de fournisseurs avec activation et sonde, modèles par agent dont Développement, dictée SAPI sélectionnable, voix et images. ComfyUI utilise le modèle du workflow. Nouveaux moteurs à essayer avec leurs configurations réelles. |
| Expérience bureau (overlay, palette, raccourcis, tray) [V][S0 §6] | 🧪 Terminé, test utilisateur à faire. Palette contextuelle dédiée, brouillon à relire, mode flottant, raccourcis globaux, menu de la zone de notification et notifications Windows. |
| Mémoire, MCP, voix, image, vidéo, workflows, multi-agents (lots D et E) | 🧪 **Mémoire 🧪** : notes chiffrées (DPAPI), mémoire de session (4 derniers échanges, jamais enregistrés, échanges locaux jamais envoyés au cloud, bouton « Nouvelle conversation »), index de fichiers choisis, index vectoriel chiffré (embeddings locaux mis en cache), recherche par l’agent dans les notes et les fichiers indexés, durée de conservation des notes et rapports. **MCP 🧪** : client bivalent 2026-07-28 / versions à poignée de main, test d’un serveur, mise à jour bloquée jusqu’à acceptation, appels toujours approuvés ; **Voix 🧪** : dictée et synthèse locales, demande vocale de bout en bout, raccourci Ctrl+Alt+V, indicateur de micro visible partout ; **Image 🧪** : deux moteurs locaux (AUTOMATIC1111, ComfyUI), format carré, paysage ou portrait, galerie des images générées, description d’une image générée par le modèle de vision local, jamais d’image vers le cloud ; **Vidéo 🧪** : travail ComfyUI asynchrone suivi et repris, workflow enregistré pour l’agent (`generate_video` sur accord), limite quotidienne réglable, annulation du seul travail suivi ; non testée faute de moteur ; **Workflows 🧪** : étapes, branches, conditions sur la réponse précédente, nouvelle tentative automatique sans répéter d’effet, déclencheurs (heure et jours, démarrage, changement du dossier de l’agent), stockés chiffrés avec les demandes enregistrées (pas de table `workflows` séparée) ; **Multi-agents 🧪** : sept agents spécialisés, agents personnalisés définis en données (rôle, base, outils, modèle, étapes) utilisables directement ou par délégation (`perso:<nom>`), délégation sans transmission d’accord ni de contexte. Tous les sous-points sont terminés ; essais réels à faire (liste ci-dessus) |
| Distribution (installateur, mises à jour) | 🧪 Installateur NSIS par utilisateur (`scripts/package.ps1`) avec pont et assistant .NET 10 autonomes embarqués par défaut, agent et Python 3.13 embarquable (`-BundlePython` / `-PythonRuntimeZip`), signature optionnelle (`AIWIN11_SIGN_THUMBPRINT`), canal de mises à jour signées GitHub (`tauri_plugin_updater` + `latest.json`) et tests unitaires Rust/React ; essai d’installation sur machine vierge et release GitHub réelle à valider |

---

## 2. Traçabilité des points de la conversation

### 2.1 Vision et capacités attendues [V]

| Point | État | Phase |
|---|---|---|
| L’IA peut faire ce que fait l’utilisateur, avec garde-fous | 🧪 45 actions typées couvertes par les profils, approbations, liste fermée, simulation, fenêtres/fichiers/apps protégés et audit (fichiers, analyse de documents Office/ODF, fenêtres, UIA, clavier ciblé `send_key`/saisie, réglages Windows `set_system_setting`, presse-papiers, navigateur `https://`, apps, processus, services, points de restauration `admin_restore_point`, périphériques, diagnostic, mémoire, MCP, image, vidéo, délégation) | R2 |
| Choisir un fournisseur local ou en ligne | ✅ Ollama, OpenAI, DeepSeek ; 🧪 LM Studio, Google Gemini, Anthropic Claude (activation, sonde, clés dans le Gestionnaire d’identifiants Windows, accord explicite par demande) | — |
| Choisir un modèle **par type de tâche** (voix, texte, image, vidéo, vision, développement, planification) | 🧪 Texte/actions, vision, voix (SAPI STT/TTS locaux sélectionnables), image (AUTOMATIC1111/ComfyUI), vidéo (ComfyUI), développement (`AgentModels.development`) et planification (`AgentModels.planning` pour les tâches planifiées) | R3 |
| Application facilement paramétrable, UI/UX soignée | 🧪 Pages Accueil, Modèles, Sécurité, Inspecteur, Fichiers, Activité, Extensions, Vidéo, Mises à jour, mode flottant et raccourcis clavier | R5 |
| Contrôle des fichiers | 🧪 Dossier dédié ou dossier choisi dans le profil, plus quatre dossiers supplémentaires (`alias/nom.txt`) ; formats texte, Markdown, CSV, JSON, YAML, XML, INI, TOML et `.log` ; binaires (PDF/Office) exclus par garde-fou | R2 |
| Contrôle des logiciels (lancer, gérer) | 🧪 Liste fermée (Bloc-notes, Paint, Table des caractères), état des applications (`list_applications`), détail (`inspect_process`) et arrêt approuvé de processus (`stop_process`) | R2 |
| Contrôle des fenêtres | ✅ Lister, premier plan, réduire, restaurer, agrandir (jamais fermer) ; lecture de la fenêtre précédente | — |
| Paramètres système | 🧪 Lecture ✅ dans `get_system_status` (thème clair/sombre, transparence, langue d’affichage, fuseau horaire, alimentation et batterie) et modification utilisateur courant (`set_system_setting` : thème clair/sombre, transparence) sur accord explicite | R2 (lecture) / R9 (modification) |
| Terminal / PowerShell | 🧪 Diagnostic PowerShell de la version Windows, en lecture seule et sur accord | R2 |
| Navigateur | 🧪 Ouverture d’une URL `https://` publique validée dans le navigateur par défaut (`open_url`, toujours sur accord) ; navigation interactive via MCP | R6 (via MCP) / R2 |
| Périphériques | 🧪 Lecture seule des périphériques Windows connectés (`list_peripherals` : audio, imprimantes, caméras, écrans, Bluetooth, saisie) | R7+ |
| Presse-papiers | ✅ Lecture et écriture de texte, accord à chaque fois (lecture réelle testée ; écriture réelle en test opt-in `AIWIN11_TEST_CLIPBOARD=1`) | — |
| Automatisations | 🧪 Workflows enregistrés (8 étapes, branches, condition sur la réponse précédente, nouvelle tentative sans effet), déclenchés par jour, au démarrage ou quand le dossier de l’agent change, en lecture seule quand personne ne regarde | R8 |
| APIs natives, puis vision + souris/clavier | 🧪 APIs natives, UIA (`inspect_window`, `find_control`, `click_control`, `set_control_value`), vision (`describe_window`, `describe_screen`) et clavier ciblé (`send_key` sur liste fermée de touches/raccourcis, `type_in_notepad`) ; souris générique non exposée par garde-fou | R2 |
| Providers locaux : Ollama ✅, LM Studio 🧪 ; cloud : OpenAI ✅, DeepSeek ✅, Anthropic 🧪, Google 🧪 | 🧪 Tous intégrés et configurables ; LM Studio, Anthropic et Google restent à valider avec des clés/instances réelles | R3 |
| Sélection manuelle **ou routage automatique** | 🧪 Quatre stratégies locales (`manual`, `adaptive`, `quality_first`, `cost_optimized`), modèle local de secours et secours cloud explicite | R3 |
| Multimodal : texte ✅, voix STT/TTS 🧪 (locales), image 🧪 (génération locale, description d’une image générée), vidéo 🧪 (ComfyUI local, travaux suivis), compréhension d’écran 🧪 (`describe_window` et `describe_screen`), analyse de documents 🧪 (`analyze_document` pour `.docx`, `.xlsx`, `.pptx`, `.odt`), temps réel 🧪 (streaming des réponses + conversation vocale continue STT/TTS) | 🧪 | R7 |
| Configuration indépendante par modalité | 🧪 Modèle d’agent, vision, voix (STT/TTS), moteurs image, vidéo, développement et planification réglés séparément | R3 |
| Agent principal qui délègue à des agents spécialisés (dev, recherche, admin Windows, bureautique, multimédia) | 🧪 Délégation d’un niveau, agents limités à leurs outils (règle `agent_scope`) | — |
| Permissions par agent, application, dossier et action | 🧪 Par action, profil, agent spécialisé (`agent_tools`), dossier de travail + 4 dossiers supplémentaires, fichiers protégés, fenêtres protégées, applications interdites et liste fermée d’applications | R2 |
| Simulations | ✅ Mode simulation | — |
| Confirmations des opérations sensibles | ✅ | — |
| Journal d’audit | ✅ | — |
| Arrêt d’urgence | 🧪 Arrêt global « Stoppe toutes les tâches » (demandé / confirmé) | — |
| Mécanismes de récupération | 🧪 Corbeille, reprise après plantage, sauvegardes des données d’AiWin11 (manuelles et automatiques avant migration, 5 gardées) avec restauration au démarrage suivant, et création de points de restauration Windows (`admin_restore_point` via assistant UAC) | R2 |
| Tableau de bord | 🧪 Centre de contrôle | — |
| Assistant flottant | 🧪 Mode flottant (petite fenêtre toujours au premier plan) | — |
| Raccourcis globaux | 🧪 Ctrl+Alt+Espace ouvre le champ de demande | — |
| Gestion des modèles | 🧪 Détection locale (Ollama/LM Studio), sélection par modalité, sonde VRAM/débit, tarifs API, modèle de secours et import/export JSON | R3 |
| Profils d’autonomie (Observation, Assisté, Autonome contrôlé, Avancé) | ✅ | — |
| Visualisation des actions en temps réel | 🧪 Étapes en direct, brouillon, journal des actions, rapport et centre de contrôle | R5 |
| Éditeur visuel de workflows | 🧪 Éditeur visuel en graphe de nœuds SVG cliquables (étapes réordonnables, conditions, reprises et branchements) | R8 |
| Écran de routage : modèle de secours, **mode privé**, copier la configuration | 🧪 Secours cloud explicite, mode privé, import/export JSON des choix de modèles | — |
| Détection des modèles installés ✅, capacités 🧪 (outils/vision sondés), conso GPU/RAM/VRAM 🧪 (VRAM et placement par modèle), coûts API 🧪 (tarif saisi par l’utilisateur), tests de performance 🧪 (débit mesuré pendant la sonde), modèles de secours 🧪 (local), restrictions de confidentialité 🧪 (mode privé, cloud explicite) | 🧪 | R3 |
| Scénario « PC lent » : examiner processus, mémoire, disque, services, proposer, appliquer les sûres, demander pour les autres, vérifier, rapport, historique | 🧪 Diagnostic en lecture seule ✅ (processus, mémoire, processeur, disques, services, périphériques), assistant guidé « PC lent » dans le Centre de contrôle, actions correctives (`admin_clean_temp`, `stop_process`, `admin_service`), vérification et rapport dans l’historique | R2 |
| Toujours protégés, même en Avancé : suppression massive, élévation de privilèges, sécurité Windows, transactions financières, transfert de données confidentielles | 🧪 Refus par défaut garanti par la liste fermée d’outils et le moteur de politiques (aucun outil de suppression massive, élévation, sécurité Windows, finance ou exfiltration) | R2 |
| Mémoire : SQLite + recherche vectorielle | 🧪 Index vectoriel dans SQLite (migration 010), vecteurs chiffrés par entrée et modèle local, supprimés avec l’entrée ou à la réindexation | R6 |
| Plugins / MCP | 🧪 Client MCP stdio bivalent (2026-07-28 et versions à poignée de main), serveurs déclarés par l’utilisateur, test depuis la page Extensions, mise à jour bloquée jusqu’à acceptation | — |
| Moteur de permissions indépendant | ✅ | — |

### 2.2 Cadrage v0.1 [S0]

| Point | État | Phase |
|---|---|---|
| 5 engagements : liberté de choix, contrôle, extensibilité, autonomie, transparence | 🧪 Liberté de choix (modèles locaux/cloud), contrôle/autonomie/transparence complets, extensibilité via MCP stdio + agents personnalisés (pas de SDK externe) | R6 |
| Fonctionnement entièrement local possible | ✅ Avec Ollama | — |
| Couches : Dashboard ✅, Chat ✅, Voice 🧪, Overlay 🧪, Settings ✅, Workflow Editor 🧪 | 🧪 | R5, R7, R8 |
| Core : Sessions ✅, Events ✅, Tasks ✅, State ✅, Configuration ✅, IPC 🧪 (Tauri + pipe gRPC `aiwin11-model-ipc`) | 🧪 | R4 |
| Context & Memory (RAG, index), Workflow Engine (triggers, jobs, scheduler) | 🧪 Mémoire chiffrée, index vectoriel et fichiers indexés consultés par l’agent ; planificateur de workflows (jours, démarrage, changement du dossier) | R6, R8 |
| Broker : Policy Engine ✅, Approvals ✅, Sandboxing 🧪 (processus agent/pont isolés, capacités u64), Audit ✅, Secrets ✅ | 🧪 | R4 |
| Stack : Tailwind CSS + composants accessibles | 🔷 CSS natif soigné avec composants accessibles sans framework (choix d’architecture) | R5 |
| Installateur signé, MSIX selon besoins | 🧪 Installateur classique autonome (`scripts/package.ps1` + vérificateur de mises à jour) | R9 |
| M01 Hub : découverte ✅, config par tâche 🧪 (`AgentModels` + modalités), profils de génération 🧪, tests de capacités ✅ (sonde d’appel d’outil/vision/débit), routage auto 🧪 (stratégies locales) | 🧪 | R3 |
| M02 Windows : fenêtres ✅, applications 🧪, fichiers 🧪, processus 🧪, interactions graphiques 🧪, capture d’écran 🧪, informations contextuelles 🧪 | 🧪 | R2 |
| M03 Agent : exécution ✅, planification ✅, outils ✅, délégation ✅, vérification 🧪, interruption ✅ (pause, reprise, annulation), **reprise** 🧪 (pause/reprise, reprise après plantage, reprise manuelle des workflows) | 🧪 | R2, R8 |
| M04 Sécurité : règles par ressource 🧪 (fichiers protégés, fenêtres protégées, applications interdites), permissions temporaires ✅, approbations ✅, journalisation ✅, arrêt immédiat ✅ | 🧪 | R2 |
| M05 Mémoire de session 🧪, préférences durables ✅, indexation sélective 🧪, historique ✅, recherche sémantique 🧪, conservation 🧪 | 🧪 | R6 |
| M06 STT/TTS 🧪, compréhension d’images 🧪 (fenêtre, écran, image générée), OCR/vision 🧪, génération image/vidéo 🧪, streaming multimodal 🧪 (réponses + conversation vocale continue) | 🧪 | R7 |
| M07 Déclencheurs ✅ (jours, démarrage) et 🧪 (changement du dossier de l’agent), planification ✅, conditions 🧪 (branches, « si la réponse précédente contient »), tâches répétitives ✅, reprise sur erreur 🧪 (nouvelle tentative automatique si rien n’a été fait, reprise manuelle), éditeur visuel 🧪 (graphe SVG en nœuds cliquables) | 🧪 | R8 |
| M08 Installation, activation, mise à jour, permissions des plugins et serveurs MCP | 🧪 Déclaration (exécutable et arguments fixes, aucun téléchargement), activation, outils autorisés, test (version, identité, outils), empreinte de l’exécutable et des fichiers passés en argument : un serveur modifié est refusé jusqu’à ce que l’utilisateur accepte la nouvelle version ; appels toujours approuvés | R6 |
| M09 App ✅, overlay 🧪, palette 🧪 (`Ctrl+Alt+Espace`), barre système 🧪, notifications et raccourcis 🧪, thème et visualisation des agents 🧪 | 🧪 | R5 |
| M10 Journaux ✅, suivi des coûts 🧪, statistiques modèles 🧪, traces d’agents ✅, ressources 🧪, diagnostics 🧪 | 🧪 | R9 |
| Ordre d’exécution : API native → API appli → UIA → vision + souris/clavier → script contrôlé ; **changer de méthode sans élargir les autorisations** | 🧪 APIs natives, UIA, recours Win32 ciblé (`send_key`), vision (`describe_window`, `describe_screen`) et PowerShell en liste fermée | R2 |
| Passerelle : modalités (dont **embeddings**), contraintes (VRAM, latence, coût, confidentialité, **quotas**), politiques de routage, détection réelle des capacités | 🧪 | R3 |
| Configuration YAML validée par schéma, générée par l’UI (`execution_mode`, `privacy_mode`, `agent_models`, `local_strategy`, `require_approval_for`) | 🧪 `yaml_config.rs` (`export_yaml_config` / `import_yaml_config`) dans **Sécurité** | R3 |
| Décision = identité + action + ressource + contexte + risque → ALLOW/DENY/ASK/SIMULATE | 🧪 Action, profil, identité d’agent, contexte, risque du manifeste (journalisé avec chaque décision) et ressource (fichiers/fenêtres/apps/MCP protégés) | R2 |
| Simulateur de politique (lire dossier, modifier document, installer logiciel, suppression massive, envoi Internet, sécurité Windows) | 🧪 Simulateur interactif dans **Sécurité** (`ALLOW`/`ASK`/`DENY`/`SIMULATE` et règle associée) | R2 |
| Isolation : processus limités ✅ (agent), environnements contrôlés / VM 🔷 (hors périmètre v0.1), pas d’exécution admin par défaut ✅ (assistant UAC à la demande) | 🧪 | R4, R9 |
| Approbation forte : fichiers, commande, conséquences, étendue | ✅ Pour les actions existantes | — |
| Anti-injection : contenu = données | 🧪 Fichiers, documents, mémoire indexée, résultats vision et fenêtres encadrés comme données non fiables ; pages web ouvertes dans le navigateur externe uniquement | R6 |
| Secrets jamais donnés aux modèles | ✅ | — |
| Arrêt d’urgence global, distinguer arrêt demandé / confirmé | 🧪 `stop_all_tasks` : arrêté, demandé puis confirmé | — |
| Récupération : points de restauration (`admin_restore_point`), sauvegardes ciblées (`Backups`), opérations réversibles (`corbeille`, `set_system_setting`) | 🧪 | R2 |
| Surfaces : Control Center ✅, Command Palette 🧪 (`Ctrl+Alt+Espace`), Voice Assistant 🧪 (demande vocale + conversation continue + `Ctrl+Alt+V`), Workflow Studio 🧪 (éditeur + graphe en nœuds) | 🧪 | R5, R7, R8 |
| UX : mode simple / mode expert | 🧪 Bouton mode expert (pages avancées masquées sinon) | — |
| UX : états d’agent toujours visibles (inactif, analyse, planification, attente, exécution, erreur) | 🧪 Indicateur permanent dans la barre latérale (inactif, analyse, planification, attente d’approbation, exécution, pause, nouvel essai, erreur), annoncé aux lecteurs d’écran | R5 |
| UX : local / cloud / transfert de données visibles | 🧪 Badge du fournisseur ; sous l’état de l’agent, « Traitement sur ce PC » ou « Données envoyées au cloud » pendant la demande (étape `model.cloud.request` avant chaque appel cloud, journalisée avec le fournisseur), puis ce qu’il en a été pour la dernière demande | R5 |
| UX : changements sans redémarrage | ✅ | — |
| UX : clavier, lecteurs d’écran, résolutions | 🟡 Navigation clavier et annonces ARIA implémentées ; audit lecteur d’écran complet à mener | R1 |
| Structure monorepo : `apps/overlay`, `packages/ui`, `provider-sdk`, `tool-sdk`, `plugin-sdk`, `plugins/`, `tests/{unit,integration,security,e2e}`, `scripts/` | 🟡 `apps/desktop` (avec overlay intégré), `services/{core,windows,admin,agent,model-ipc}`, `contracts/`, `scripts/` ; `packages/` et SDK externes non créés | R4, R6 |
| Contrat d’outil : id, version, capacités, schéma, **risque, dry-run, réversible, timeout** | ✅ `manifest.rs` : un manifeste par outil, vérifié contre le catalogue et l’agent ; les invariants de politique en dérivent ; affiché dans **Sécurité** | — |
| Base MCP spec 2026-07-28 | 🧪 Sonde `server/discover`, version et capacités dans le `_meta` de chaque requête, `resultType`, erreur -32022 ; repli sur `initialize` (2025-11-25 à 2024-11-05) pour un serveur ancien | R6 |
| Critères NFR et scénarios | Voir §3 et §4 | — |
| Roadmap phases 0 à 5 | Voir §5 | — |
| Licence : open source, privée ou hybride | ✅ Sans objet (usage personnel) | — |
| Service privilégié facultatif | 🧪 Assistant élevé à la demande (option B) : `AiWin11.Admin.exe`, liste fermée (services, fichiers temporaires), désactivé par défaut, accord + UAC, empreinte SHA-256 vérifiée et fichier verrouillé pendant l’opération (build release) ; 🟡 UAC réel non testé, signature de code optionnelle (`AIWIN11_SIGN_THUMBPRINT`, certificat à fournir) 🟡 | — |

### 2.3 Spécification v1.0 [S1]

| Point | État | Phase |
|---|---|---|
| 6 processus (desktop, core, broker, windows, agent, model) | 🧪 Six rôles isolés, démarrés à la demande. Tauri conserve les préférences de bureau et le rendu. Le broker exécute les commandes Windows, y compris celles de l’Inspecteur. | R4 |
| Admin futur via composant dédié limité en durée et périmètre | 🧪 Assistant élevé à la demande (option B), une opération par lancement | — |
| Frontend sans terminal ni système de fichiers direct, commandes Tauri restreintes | ✅ Capacités Tauri par commande | — |
| Transport : Tauri IPC pour le WebView, gRPC sur canaux nommés réservés au compte courant avec vérification mutuelle des PID. Protobuf sur flux privés pour le pont et HTTP en boucle locale pour Ollama. | 🧪 | R4 |
| Enveloppe avec `payload`, identité attribuée par une couche de confiance | 🧪 Enveloppes `Command`, `Result` et `Event` (`oneof` typé) ; identité et horodatage fixés par l’hôte | R4 |
| Réponse normalisée (statuts, erreur code/message/retryable, durée) | ✅ Contrat v1 | — |
| Événements : identifiant, séquence, horodatage, session, tâche, type et source. L’export JSON décode les payloads connus à partir des codes d’audit. | ✅ | R1 |
| Types `task.started`, `task.progress`, `model.provider.unavailable` | ✅ | — |
| Ordre par tâche ✅ ; tolérance aux doublons après reconnexion | 🧪 Déduplication par `request_id` (`broker.rs`) et reprise/reconnexion sans double exécution (`task_host.rs`) | R4 |
| Model Gateway : ProviderRegistry ✅, ModelCatalog ✅, ModelRouter (Resolve, Rank, Select, ExplainSelection 🧪), InferenceService (Generate, Stream ✅, Cancel ✅), ResourceMonitor ✅, UsageService ✅ | 🧪 | R3 |
| Manifeste de modèle (modalités, capacités, matériel, limites, statut `unverified`) | 🧪 table `models` (migration 007) : appels d’outils `unverified`/`not_advertised`/`verified`/`failed`, vision, durée et date du dernier test ; fenêtre de contexte, nombre de paramètres, quantification et taille sur disque (migration 008), débit mesuré pendant le test et VRAM utilisée une fois chargé (migration 009) ; modèles en échec exclus du routage | R3 |
| Stratégies : manual, local_only, cloud_only, local_first, quality_first, cost_optimized, adaptive et mixte. Le classement mixte exige des appels d’outils vérifiés et un accord explicite avant le cloud. | 🧪 | R3 |
| Aucun repli local → cloud en mode privé strict | 🧪 `private_mode` bloque tout appel et repli cloud dans le noyau et la passerelle | R3 |
| HAL NVIDIA/AMD : GPU, pilotes, VRAM, backend, modèles chargés, repli CPU, diagnostic d’incompatibilité | 🟡 Cartes, pilote et mémoire (WMI) ✅, modèles chargés et VRAM ✅, placement par modèle (GPU, partiel, processeur seul) avec conseil ✅ ; backend d’inférence (CUDA/ROCm) non exposé par WMI | R3 |
| Agent défini en YAML (rôle, profil de modèle, capacités, limites, vérification, `retry_limit`, profil de politique) | 🧪 Agents personnalisés définis en données (`CustomAgents.tsx` / `agents.rs`) + configuration YAML schéma v1 (`yaml_config.rs`) | R8 |
| États : CREATED ✅, PLANNING ✅, RUNNING ✅ (`dispatching`), AWAITING_APPROVAL ✅, COMPLETED ✅, FAILED ✅, CANCELLED ✅, **PAUSED** ✅, **RECOVERING** ✅ (nouvel essai unique des lectures système), CANCELLING 🧪 (arrêt demandé puis confirmé) ; `interrupted` après un plantage | ✅ | — |
| Cycle Understand → Plan → Authorize ✅ → Execute ✅ → Observe ✅ → Verify 🧪 → Recover 🧪 (un nouvel essai des lectures système après un échec passager du pont ; jamais d’effet rejoué) → Report ✅ | 🧪 | R2 |
| Contrat d’exécution d’outil avec `dry_run`, `timeout`, `idempotency_key` ; retour avec décision, effets, observations, statut de vérification | 🧪 `manifest.rs` + `broker.rs` + `Result` + journal d’audit | R2 |
| Broker : EvaluateAction ✅, RequestApproval ✅, GrantCapability / RevokeCapability temporaires ✅ (table `grants`), ExecuteAuthorized ✅, CancelExecution 🧪 (`stop_all_tasks` / annulation), GetAuditTrail ✅ (export) | 🧪 | R2 |
| Politique JSON (sujet, actions, **ressources/chemins**, contraintes `max_files`, `require_backup`, `expires_after`) | 🧪 `export_policy_json` (`yaml_config.rs`) dans **Sécurité** | R2 |
| Chemins canoniques, jonctions, liens | ✅ Fichiers | — |
| Invariants : action inconnue refusée ✅, pas d’auto-attribution ✅, confirmation précise ✅, pas de transmission entre agents ✅, LLM jamais autorité ✅, erreur → refus ✅, contournement = défaillance ✅, révocation effective ✅, vérification juste avant exécution ✅ | 🧪 | R2 |
| Windows Bridge — WindowService (List ✅, Focus ✅, Minimize/Restore/Maximize ✅, Resize 🧪 moitié gauche/droite ou centrée aux deux tiers de l’écran de la fenêtre, Close ❌ volontairement absent) | 🧪 | R2 |
| ApplicationService (Launch ✅ liste fermée : Bloc-notes, Table des caractères, Paint ; Discover et GetStatus 🧪 `list_applications` : installée, interdite, processus et fenêtres ouverts) | 🧪 | R2 |
| FileService (List ✅, Read ✅, Write 🧪, Copy ✅, Move ✅, Delete 🧪 corbeille, Search ✅, Document 🧪 `analyze_document`) | 🧪 | R2 |
| ProcessService (List ✅ lecture seule, Inspect 🧪 `inspect_process` sur un processus de la dernière liste, Terminate 🧪 `stop_process` : accord à chaque fois, fermeture demandée sans forcer pour un programme avec fenêtre, processus système, de sécurité et d’AiWin11 refusés) | 🧪 | R2 |
| DesktopService (CaptureScreen 🧪, GetDisplays ✅, GetActiveWindow ✅) | 🧪 Capture d’une fenêtre ✅ ; écran principal pour l’agent 🧪 (`describe_screen`, vision locale, refus si une fenêtre protégée est visible) ; écrans et capture d’un écran choisi dans l’Inspecteur | R2 |
| UIAutomationService (InspectTree ✅, Invoke ✅ et SetValue ✅ sur un contrôle numéroté de la fenêtre lue, FindElement 🧪 `find_control` par nom ou type dans la fenêtre lue) | 🧪 | R2 |
| InputService (Click ✅ UIA/Win32, Type ✅ Bloc-notes, Hotkey 🧪 `send_key` sur liste fermée ; déplacement générique du pointeur volontairement absent) | 🧪 | R2 |
| ShellService (ExecuteApprovedCommand) | 🧪 Diagnostics réseau, Wi-Fi, heure et PowerShell version Windows en liste fermée, sur accord | R2 |
| SystemService (GetSystemMetrics ✅ mémoire, processeur, disques, durée, thème/langue/fuseau/batterie ; GetHardwareInfo 🧪 `list_peripherals` + WMI GPU) | 🧪 | R2, R3 |
| DPI, multi-écran, défilement, focus, références UIA périmées, dialogues modaux | 🟡 Redimensionnement multi-écran et focus implémentés ; matrice de tests DPI/modales réels à valider sur machine cible | R2 |
| Capture jamais envoyée au cloud sans politique | ✅ Vision locale seulement | — |
| Services : Memory ✅, Workflow ✅, Speech ✅, Image 🧪, Video 🧪, Plugin 🧪 (MCP), Telemetry locale ✅ (journal) | 🧪 | R6–R9 |
| MCP avec passerelle de sécurité ; autorisation MCP ≠ permission AiWin11 | ✅ `mcp_call` passe par le broker et est toujours demandé | — |
| Tables : sessions ✅, tasks ✅, tool_executions ✅, events ✅, settings ✅, models 🧪, grants ✅, memory_entries ✅, memory_vectors 🧪, mcp_servers 🧪, backups 🧪 | 🧪 | R2, R3, R6, R8 |
| Dates UTC ✅, migrations versionnées ✅, clés hors SQLite ✅, chiffrement : pas de chiffrement de la base, rien de sensible stocké (✅ décidé, §6), durée de conservation et effacement ✅ | ✅ | — |
| Profil IA : stratégie, capacités requises, `allow_cloud_fallback`, `allow_screen_upload`, `allow_document_upload`, **budget par tâche** | 🧪 `AiProfile` (`lib.rs`, `Routing.tsx`), bloqué par le mode privé | R3 |
| Lots A ✅, B ✅, C 🧪, D 🧪, E 🧪 | ✅ | — |
| Matrice de tests T-001 à T-014 | Voir §3 | — |
| Exigences de performance | Voir §4 | — |
| Risques : UIA ✅, GPU 🟡, actions incorrectes ✅, injection ✅, suppression ✅ (corbeille), fuite cloud ✅, fragmentation providers ✅, conso GPU ✅, dépendance frameworks ✅ | 🧪 | R2, R3 |
| Signaler les fenêtres élevées ou protégées au lieu de contourner | 🧪 Affinité d’affichage refusée + détection explicite des fenêtres élevées (`IsProcessElevated` -> `window_elevated` dans `KeyInput.cs`) | R2 |
| À valider par prototype : perf Rust/.NET/Python, IPC définitif, stabilité UIA, capture, packaging, empreinte mémoire | 🧪 IPC pipe gRPC, packaging autonome `scripts/package.ps1` et chronométrage démarrage implémentés ; benchmarks complets §4 sur machine cible | R1, R4, R9 |
| À adapter au matériel : taille des modèles, accélération, modèles simultanés, quantification, latence, VRAM | 🧪 Manifestes (taille, quantification, contexte, débit, VRAM) et placement GPU/partiel/CPU par modèle avec stratégie adaptative | R3 |

### 2.4 Transmission [H] et règles du dépôt (AGENTS.md)

| Point | État | Phase |
|---|---|---|
| Dépôt privé pour versionner le travail de Codex | ✅ Dépôt GitHub `quassim/aiwin11` (branche `main`), poussé à la demande de l’utilisateur le 13/11/2026 ; `scripts/verify.ps1` reste la vérification de référence | — |
| CI sur un vrai runner Windows 11 | 🟡 Le workflow GitHub Actions (`.github/workflows/windows.yml`) cible `windows-2025` (runner standard GitHub), et `scripts/verify.ps1` est exécuté sur le poste Windows 11 de référence | R1 |
| Code linté et typé | ✅ `clippy -D warnings`, `cargo fmt --check`, TypeScript strict, ESLint + Prettier, `ruff` + `mypy` strict, et analyseurs .NET (`EnableNETAnalyzers` / `AnalysisLevel=latest-recommended` + `TreatWarningsAsErrors` sur `AiWin11.Windows` et `AiWin11.Admin`), tous dans `scripts/verify.ps1` | — |
| Tests unitaires, intégration, sécurité | ✅ 174 tests Rust (`aiwin11-core` : 161 + 24 réels/ignorés, `aiwin11-desktop` : 2, `aiwin11-model-ipc` : 11), 28 tests Python, 16 tests Vitest | — |
| Aucune opération hors broker | ✅ | — |
| APIs publiques et migrations documentées | ✅ `docs/CONTRACTS.md` | — |
| README : installation, lancement, dépannage | ✅ | — |
| Pas de télémétrie, pas de serveur exposé au LAN | ✅ | — |
| UI accessible, badges local/cloud, activité de l’agent | ✅ `role="status"` et `aria-live="polite"` sur l’état de l’agent et les panneaux, `aria-current="page"` dans la navigation, badges `Local` / `Cloud` (`transfer-cloud`), tests Vitest (`agentState.test.ts`, `components.test.tsx`) | R1, R5 |

### 2.5 Points de la conversation ajoutés après relecture complète (12/11)

| Point | Source | État | Phase |
|---|---|---|---|
| Limites d’un agent : `max_steps` 30, `max_tool_calls` 50, `max_runtime_seconds` 900, `max_parallel_tools` 2, `retry_limit` 2, `verification_required` | [S1 §5.1] | ✅ `MAX_AGENT_STEPS` 30, `MAX_TOOL_CALLS` 50 (`AGENT_TOOL_LIMIT`), `MAX_TASK_RUNTIME` 900 s (`AGENT_TIME_LIMIT`), une action vérifiée à la fois, reprise `[réessayer]` sur les routines (`task.rs`, `CustomAgents.tsx`) | R8 |
| Résilience : la panne d’un provider ou d’un agent ne fait pas tomber le noyau ; « crash d’un agent : core et interface restent fonctionnels » | [S0 §8][S1 §11] | 🧪 Agent Python, pont, assistant et serveurs MCP en processus séparés avec délais ; le verrou du cœur se rétablit après une panique ; test d’injection de panne Python (`crashing_agent_process_fails_cleanly_and_leaves_core_healthy` dans `task.rs`) | R4, R9 |
| Plafond de coût par tâche (`max_cost_usd_per_task`), vérification du budget avant chaque appel cloud | [S1 §9] | 🧪 Tarif par million de jetons et plafond par demande saisis par l’utilisateur (aucun prix intégré), coût estimé vérifié avant chaque appel cloud (`CLOUD_COST_EXCEEDED`), en plus du plafond de jetons | R3 |
| Mode privé / local strict : aucun transfert cloud, jamais de bascule local→cloud | [V][S0 §8][S1 §4.3], T-011 | 🧪 Interrupteur **Mode privé** (Sécurité), aucun fallback automatique | — |
| Intégration profonde : agent permanent, interfaces contextuelles, événements Windows, automatisation proactive | [V §7] | 🟡 Mode vocal continu (`voiceMode`) et routines enregistrées en place ; déclencheurs système Windows proactifs non implémentés | R5, R8 |
| Zero-trust entre agents et outils ; un agent ne transmet pas d’autorisation à un autre | [S0 §10][S1 §6.3] | ✅ pour l’agent unique (broker, identité attribuée par le cœur) ; 🧪 multi-agents : un sous-agent repart sans accord, autorisation ni contexte de la tâche, ne peut que restreindre sa base et ne délègue jamais | R8 |
| Périphériques, terminal, navigateur parmi les capacités | [V §2] | 🟡 Écrans, GPU et audio en lecture (`get_system_status`, dictée locale) ; aucun terminal libre autorisé (`AGENTS.md`) ; navigateur via MCP prévu | R2, R6 |
| Agents spécialisés (développement, recherche, administration Windows, bureautique, multimédia) | [V §2] | 🧪 Général, Fichiers et bureautique, Administration Windows, Développement, Recherche, Multimédia, Lecture seule, chacun limité à ses outils (règle `agent_scope`) ; agents personnalisés (`perso:<nom>`) ; délégation d’un niveau par l’agent Général ✅ | R8 |
| Isolation des tâches risquées (processus limités, environnement contrôlé, VM si pertinent) | [S0 §5] | 🟡 Processus séparés, utilisateur standard, assistant élevé une opération ; pas de bac à sable ni de VM | R4 |
| Licence (open source, privée, hybride) | [S0 §10] | 🔷 À décider (usage personnel pour l’instant) | — |
| Coûts API, consommation GPU/RAM/VRAM et tests de performance par modèle dans le gestionnaire de modèles | [V §4] | 🧪 Débit (`jetons/s`), VRAM, taille, placement par modèle et banc de performance multi-modèles (`ModelProbe.tsx`) ; coût estimé à partir du tarif saisi | R3, R9 |

---

## 3. Scénarios et matrice de tests [S0 §8][S1 §11]

| Réf. | Scénario / test | État |
|---|---|---|
| — | « Ouvre le Bloc-notes » | ✅ Réel |
| — | « Résume cette fenêtre » | ✅ Réel (structure et description locale) |
| — | « Range ces fichiers » : **plan prévisualisé**, puis actions contrôlées | 🧪 `organize_files` : plan de renommages vérifié, montré en entier, approuvé en une fois (non testé en réel) |
| — | « Utilise mon modèle local » : aucun appel cloud | ✅ |
| — | « Stoppe toutes les tâches » | 🧪 Arrêt global (tests unitaires ; réel non testé) |
| — | Provider indisponible : bascule autorisée ou erreur compréhensible | 🧪 Erreur contrôlée (`MODEL_UNAVAILABLE_OR_INVALID`), bascule locale de secours vérifiée par sonde (`T-009`), bascule cloud uniquement sur accord explicite hors mode privé |
| — | Instruction malveillante dans une page | 🟡 Fichiers ✅ ; fenêtres/observations encadrées (`<<<DEBUT ... FIN>>>`) 🧪 ; navigation web via MCP non activée — R6 |
| — | Redémarrage après interruption | ✅ |
| T-001 | Démarrage à froid, services démarrés et états visibles | 🟡 Release mesurée sous Windows 11. Premier rendu à chaud en 333 à 340 ms depuis la création du processus, sur trois essais. Première fenêtre observée à 3,51 s lors du premier lancement de la série précédente. Mesure du premier rendu après démarrage à froid de Windows à faire. |
| T-002 | Provider local | ✅ |
| T-003 | Provider cloud | ✅ DeepSeek réel via gRPC et via le parcours dans la release Windows. Test OpenAI disponible séparément avec `verify.ps1 -CloudVendor openai -CloudModel <modèle>` si une clé OpenAI est configurée. |
| T-004 | Changement de modèle sans redémarrage | ✅ |
| T-005 | Action Windows simple vérifiée | ✅ |
| T-006 | Action non autorisée refusée et journalisée | ✅ |
| T-007 | Aucune exécution avant approbation | ✅ |
| T-008 | Annulation | ✅ Processus gRPC et agent Python réels. Annulation pendant une approbation, refus de l’ancienne décision et absence de copie sur un fichier de test. |
| T-009 | Panne du modèle | 🟡 Erreur contrôlée ; modèle local de secours configurable, utilisé une fois si le modèle choisi échoue avant la première étape (après un appel d’outil de test réussi) ; non testé avec Ollama réel — R3 |
| T-010 | Injection de prompt | 🧪 Tests ciblés sur les réponses, les autorisations injectées, le multi-outil furtif (`test_rejects_multi_tool_smuggling_and_injected_approval`), les observations encadrées (`prompt_injection_in_observations_and_mcp_outputs_are_framed`) et les chemins hors dossier |
| T-011 | Mode privé : aucun transfert réseau cloud | 🧪 Interrupteur Mode privé vérifié avant tout appel réseau (`private_mode_blocks_cloud_routing_and_fallback_before_network` → `PRIVATE_MODE` + `model.cloud.denied`) |
| T-012 | Reprise après crash | ✅ |
| T-013 | Multi-écran / DPI | 🟡 `Displays.cs` et `WindowCapture.cs` utilisent `PerMonitorAwareV2`, `windows_bridge.rs` valide les coordonnées multi-écrans négatives (`display_list_accepts_negative_secondary_coordinates_and_rejects_zero_dimensions`) ; test matériel multi-DPI en réel à faire |
| T-014 | Détection GPU NVIDIA/AMD | 🟡 Cartes, pilote et mémoire déclarée (WMI) dans `get_system_status` ; VRAM utilisée par modèle via Ollama ; repli CPU détecté par modèle (placement) ; backend d’inférence non diagnostiqué — R3 |

## 4. Exigences non fonctionnelles [S0 §8][S1 §11]

| Exigence | État |
|---|---|
| Palette p95 < 200 ms | ✅ 44 ms sur 22 ouvertures de la release Windows locale — R5 |
| Évaluation d’une permission p95 < 50 ms | ✅ p95 7,1 ms, p50 6,6 ms (politique + autorisation du broker journalisée sur SQLite disque, 500 essais, release, machine de référence, 12/11/2026) |
| Navigation p95 < 150 ms | ✅ 31 ms sur 24 transitions Accueil ↔ Modèles, répété sur deux séries release sous Windows 11. |
| Interface prête < 3 s | 🟡 333 à 340 ms sur trois lancements release à chaud. Mesure du premier rendu à froid à faire. |
| RAM au repos < 350 Mo hors modèles | ✅ 178–182 Mo de mémoire privée pour 9 processus (application, WebView2, pont de réserve) après 15 s ; ensemble de travail cumulé 358–366 Mo, qui compte plusieurs fois les pages partagées de WebView2 |
| Crash d’un agent : cœur et UI fonctionnels | ✅ L’agent Python est un processus séparé, borné dans le temps (`crashing_agent_process_fails_cleanly_and_leaves_core_healthy`) |
| 100 % des appels d’outils par le broker, 100 % des actions tracées | ✅ |
| Aucune requête cloud en mode local strict | ✅ Par conception et test (`private_mode_blocks_cloud_routing_and_fallback_before_network`) |
| Changement de provider sans redémarrage global | ✅ |
| Machine de référence définie | ✅ PC de l’utilisateur (i9-12900K, 32 Go, RTX 3090, Windows 11 Pro) — §6 |

---

## 5. Phases

L’ordre suit le principe de la conversation : sécurité et fiabilité avant l’autonomie avancée, une version testable à chaque étape. Les phases R1 à R3 terminent la « Phase 1–2 » de la roadmap initiale [S0 §9] ; R4 à R9 couvrent les phases 3 à 5.

### R1 — Consolider le produit

- ✅ `scripts/start.ps1` publie les compagnons et démarre le bureau. Le cœur détecte Python sur le PATH et accepte une surcharge facultative.
- ✅ Réglages mémorisés : fournisseur, modèle, modèle de vision (table `settings`, événement `settings.models.changed`).
- ✅ **Arrêt global** (« Stoppe toutes les tâches ») avec état demandé / confirmé ; bouton toujours visible.
- ✅ **Durée de conservation et effacement** de l’historique ; export du journal d’audit (JSON).
- ✅ Événements de tâche et de fournisseur. L’export ajoute un payload structuré pour les fournisseurs connus, les numéros d’étape et les durées de rendu. SQLite conserve les codes.
- ✅ Qualité : linters, tests React, Rust, Python et .NET dans `verify.ps1`. `scripts/smoke-desktop.ps1` couvre la saisie, le consentement cloud, la proposition d’action, le refus et le journal dans la release Windows.
- 🟡 Mesures release avec `scripts/measure.ps1`. Navigation p95 à 31 ms. Premier rendu à chaud en 333 à 340 ms, mesure à froid à faire. Au repos, mémoire privée agrégée de 201 à 204 Mio et working set agrégé de 374 à 377 Mio sur la dernière série.
- ✅ T-008 sous Windows et premiers cas d’injection AW-024. T-003 validé avec le fournisseur configuré, DeepSeek.
- 🧪 UI Automation a vérifié le focus du lien d’évitement et du bouton **Refuser**. Avec Narrateur ou NVDA, vérifier l’annonce du titre, de la demande d’accord et du changement d’état après refus.
- ✅ `scripts/verify.ps1 -Ipc -Tauri` sert à la vérification locale et au workflow GitHub Actions sur Windows Server 2025.
- ✅ Test qui garantit l’invariant « rien de sensible dans SQLite » (voir §6) : schéma revu colonne par colonne, garde `journal_code` qui remplace tout texte libre par `REDACTED`, scan du fichier de base.

### R2 — Politiques par ressource et contrôle Windows étendu (fin du lot C, début de M02/M04)

- 🟡 Politiques en données : sujet, action, **dossiers autorisés** (`folders.rs`, `ExtraFolders.tsx`) ✅, **applications** (`BlockedApps.tsx`) ✅, **autorisations temporaires et révocables** (`grants.rs`, `Grants.tsx`) ✅, simulateur et profils IA (`Security.tsx`, `yaml_config.rs`, `export_policy_json`) 🧪 ; table `policies` dédiée et contraintes avancées (`max_files`, `require_backup`) partielles (`🟡`).
- ✅ Risque formel par outil et **manifeste d’outil** (risque, dry-run, réversible, timeout, capacités) dans `manifest.rs`.
- ✅ Règles critiques explicites (toujours demandées ou refusées) : suppression massive refusée, élévation/administration (`admin.rs`), fichiers protégés (`ProtectedFiles.tsx`), envoi cloud en mode privé refusé (`PRIVATE_MODE`).
- Pont Windows :
  - 🧪 ApplicationService : découverte et état (`list_applications`) et lancement ✅ des applications autorisées ;
  - 🟡 WindowService : lister ✅, focus ✅, réduire/restaurer/agrandir ✅ ; redimensionner 🧪 (dispositions sur l’écran de la fenêtre) ; fermer ❌ (risque de perte de données, à décider) ;
  - 🧪 ProcessService : lister ✅, inspecter 🧪, arrêter (avec approbation) 🧪 ;
  - 🧪 UIAutomationService : `inspect_window` (arbre UIA) et `click_control` (`ControlInput.cs`) sur contrôle Win32 inspecté ;
  - 🟡 InputService : clic et saisie sur un contrôle Win32 inspecté 🧪 ; raccourcis clavier (`send_key`) 🧪 ; déplacement libre du pointeur ❌ ;
  - 🧪 ShellService : diagnostic PowerShell local depuis une liste fermée, sur accord ;
  - ✅ Presse-papiers (lecture et écriture avec approbation) ;
  - 🧪 SystemService : métriques (mémoire, processeur, disques), périphériques (`list_peripherals`) et matériel (`get_system_status`) ;
  - 🧪 DesktopService : écrans (`Displays.cs`, `PerMonitorAwareV2`), capture de fenêtre (`WindowCapture.cs`).
- ✅ **Assistant élevé à la demande** (option B, §6) pour les opérations administrateur d’une liste fermée (incluant `admin_restore_point`) : double validation AiWin11 + UAC, une opération par lancement, puis arrêt.
- 🟡 Changement de méthode (API → UIA → vision → clavier) **sans élargir les autorisations** : chaque étape passe par le broker avec son propre accord.
- 🧪 Fichiers : plusieurs dossiers autorisés (`ExtraFolders.tsx`), recherche (`search_text_files`), formats bureautiques (`analyze_document` sur `.docx/.xlsx/.pptx/.odt`), corbeille réversible, sauvegardes (`backup.rs`, `Backups.tsx`).
- 🧪 Scénarios « Range ces fichiers » (`organize_files`, plan prévisualisé, approbation groupée) et « PC lent » (assistant guidé dans `Inspector.tsx` : processus, mémoire, disque, services, actions sûres, rapport, historique).
- 🧪 États `PAUSED`, `CANCELLING`, `RECOVERING` ; étapes de vérification par outil et reprise (`[réessayer]`).
- 🟡 Tests T-013 (validation multi-écrans `display_list_accepts_negative_secondary_coordinates_and_rejects_zero_dimensions` 🧪, test matériel multi-DPI à faire), fenêtres élevées signalées (`window_elevated` dans `KeyInput.cs` 🧪).

### R3 — Hub de modèles (M01, AW-020)

- 🧪 Registre de fournisseurs (`model_gateway.rs`, `openai.rs`), manifestes des modèles locaux (`ModelProbe.tsx`) avec statut `unverified` jusqu’à la sonde vérifiée.
- 🧪 Modèles Ollama par agent et délégation, profil Développement, modèle de vision et d’embeddings, moteur SAPI, voix locale et modèle d’images. ComfyUI utilise le modèle du workflow.
- 🧪 Stratégies `manual`, `local_only`, `cloud_only`, `local_first`, `quality_first`, `cost_optimized`, `adaptive` et `AiProfile`. La tâche indique le modèle retenu et le motif.
- 🧪 **Mode privé** explicite (`private_mode_blocks_cloud_routing_and_fallback_before_network`), repli autorisé ou non, jamais de repli local → cloud en mode strict (T-011).
- 🧪 LM Studio en boucle locale, Gemini et Claude via leurs interfaces de compatibilité Chat Completions. Le registre permet d’activer les fournisseurs cloud et de sonder leurs appels d’outils.
- 🟡 HAL GPU NVIDIA/AMD : cartes, pilote et mémoire déclarée (WMI) dans `get_system_status`, VRAM utilisée par modèle et repli CPU détectés via Ollama (`placement`), diagnostic backend d’inférence à compléter (T-014).
- 🧪 UsageService : coûts estimés à partir du tarif saisi (`CloudCost.tsx`), **budget par tâche** (`CLOUD_BUDGET_EXCEEDED`, `CLOUD_COST_EXCEEDED`) vérifié avant chaque appel, quotas.
- ✅ Configuration exportable/copiable (`ModelConfig.tsx` JSON, `yaml_config.rs` YAML v1 validé par schéma, `export_policy_json`).
- 🧪 Tests de capacités, débit (`jetons/s`) et banc de performance multi-modèles (`ModelProbe.tsx`).

### R4 — Architecture multi-processus (ADR-002, ADR-007)

- ✅ Prototype gRPC sur canal nommé Windows, limité au compte courant, avec vérification mutuelle des PID. `scripts/verify.ps1 -Ipc` couvre le protocole, les PID erronés et l’arrêt du processus enfant.
- 🧪 Catalogue Ollama : découverte et ressources des modèles en cours d’exécution dans un processus Rust lancé à la demande. Vérifier les deux listes dans la page Modèles avec Ollama en cours d’exécution.
- 🧪 Routage des tâches Ollama en processus séparé pour les stratégies adaptative, qualité et coût. Le processus s’arrête si la tâche est annulée ou si le parent ferme son flux de supervision.
- 🧪 Inférence des tâches Ollama en processus séparé, avec flux des fragments et proposition finale. La validation des outils, les approbations et l’exécution passent toujours par le cœur et le broker.
- 🧪 Inférence OpenAI et DeepSeek dans le processus de modèles. La clé reste dans le Gestionnaire d’identifiants Windows et le cœur conserve les contrôles de confidentialité, de budget et de coût.
- 🧪 Reprise du journal d’activité après interruption de la connexion. Le service de lecture ouvre SQLite en lecture seule et reprend à la dernière séquence reçue. La vue Activité déduplique les entrées.
- 🧪 Le cœur initialise la session, applique les migrations ou la restauration et orchestre les tâches hors de Tauri. Une panne de tâche la marque comme interrompue. La suivante démarre un nouveau processus.
- 🧪 Le broker indépendant autorise, réserve et journalise les commandes. Il exécute les commandes Windows via le pont .NET. Les adaptateurs fichiers et administrateur exécutent l’action approuvée dans leur processus d’origine.
- ✅ Validation stricte des propositions de l’agent dans le runtime Python (`services/agent/agent.py`), le broker restant seul juge.
- 🧪 Reprise des fragments et des approbations par curseur tant que le superviseur Tauri tourne. Le test Windows coupe gRPC pendant une approbation puis vérifie une seule copie de fichier après reconnexion.
- 🟡 Processus séparés en utilisateur standard, limites de ressource/temps (`MAX_AGENT_STEPS = 30`, `MAX_TOOL_CALLS = 50`, `MAX_TASK_RUNTIME = 900 s`) ; bac à sable OS / VM complet non implémenté.

### R5 — Expérience bureau (M09, AW-022)

- 🧪 Overlay contextuel, assistant flottant, palette de commandes, raccourcis globaux, zone de notification et notifications Windows. À vérifier en usage réel sur plusieurs écrans et avec les notifications Windows activées. Test ciblé `scripts/smoke-desktop.ps1 -PaletteOnly`.
- 🧪 Tableau de bord (`ControlCenter.tsx`) : modèles, agents, permissions, tâches, ressources.
- ✅ Mode simple / mode expert ; **état de l’agent toujours visible** (`role="status"`, `aria-live="polite"`) ; indicateur local / cloud / transfert de données.
- 🔷 Design system (CSS accessible dédié en place aujourd’hui ; migration Tailwind ou alternative à décider si souhaitée).

### R6 — Mémoire, extensions et MCP (M05, M08, AW-018, AW-019)

- 🧪 Mémoire de session, préférences durables, notes mémorisées (`memory.rs`, `Memory.tsx`), classement par embeddings Ollama locaux ou mots-clés, suppression et rétention.
- 🧪 Client MCP derrière le broker (`mcp.rs`, `Extensions.tsx`) avec vérification d’intégrité des exécutables, filtrage des outils restreints et gestion des extensions.
- 🟡 Observations encadrées comme données non fiables (`<<<DEBUT ... FIN>>>`) et tests AW-024 sur fichiers/fenêtres (`prompt_injection_in_observations_and_mcp_outputs_are_framed`) ; navigation web MCP dédiée non activée.

### R7 — Multimodal (M06, AW-021, AW-027, AW-028)

- 🧪 Voix : STT (`Speech.cs`) et TTS (`Speech.tsx`) locaux, écoute explicite avec indicateur visible et mode conversation vocal continu (`voiceMode`).
- 🧪 Analyse de documents bureautiques (`analyze_document` sur `.docx/.xlsx/.pptx/.odt`) ; capture et description locale de fenêtre (`WindowCapture.cs`, `describe_window`).
- 🧪 Génération d’images locale (`images.rs`, `Gallery.tsx`) ; jobs vidéo asynchrones locaux ComfyUI (`video.rs`, `Video.tsx`) avec suivi et quotas.
- 🧪 Contrôle des transferts : aucune image ni document envoyé vers le cloud en mode privé ou sans politique explicite (`allowScreenUpload`, `allowDocumentUpload` dans `AiProfile`).

### R8 — Workflows et multi-agents (M07, AW-023, AW-025, AW-026, AW-029)

- 🧪 Workflows multi-étapes (`Routines.tsx`), planification, branchements conditionnels (`[si échec]`, `[suite:...]`), reprise sur erreur (`[réessayer]`).
- 🧪 Agents spécialisés (`General`, `Files`, `Admin`, `Dev`, `Research`, `Media`, `ReadOnly`) et agents personnalisés (`agents.rs`, `CustomAgents.tsx`), chacun limité à sa portée (`agent_scope`) ; délégation d’un niveau sans transmission d’autorisations.
- 🧪 Éditeur visuel de workflows (`WorkflowGraph` SVG interactif dans `Routines.tsx`) ; automatisations événementielles système Windows proactives ❌.

### R9 — Observabilité et distribution (M10, AW-024, AW-030 à AW-032)

- 🧪 Journaux techniques (`History.tsx`), traces d’agents, statistiques des modèles, ressources, diagnostics et suivi des coûts saisis (`CloudCost.tsx`).
- 🧪 Suite de tests de sécurité et de robustesse AW-024 (`task.rs`, `broker.rs`, `test_agent.py`, `crashing_agent_process_fails_cleanly_and_leaves_core_healthy`).
- 🧪 Gestionnaire local d’extensions MCP (`Extensions.tsx`, `mcp.rs`) ; catalogue distant de plugins ❌.
- 🧪 Installateur NSIS par utilisateur généré avec `scripts/package.ps1 -BundlePython`. La configuration `tauri.bundle.json` active l’empaquetage. Essai sur une machine vierge et canal de mises à jour signé à valider.

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
