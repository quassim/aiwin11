# AiWin11

Application personnelle pour Windows 11 x64. Une interface Tauri/React dialogue avec un modèle (Ollama en local, ou OpenAI en option), un agent Python valide chaque action proposée, un moteur de politiques et un broker Rust décident de ce qui s’exécute, et un pont .NET agit sur Windows. Tout est journalisé dans SQLite.

## Démarrer sous Windows 11

**Raccourci (machine déjà préparée)** : double-clique sur `Ollama.cmd` (laisse la fenêtre ouverte), puis sur `AiWin11.cmd`. Ou dans PowerShell : `scripts\ollama.ps1` puis `scripts\start.ps1`. Détail ci-dessous.

Installez Node.js 24 avec npm, Rustup avec la cible MSVC x64, les outils Visual Studio C++ (charge de travail « Développement Desktop en C++ »), WebView2 et le SDK .NET 10 pour le pont Windows. Rustup lit la version de Rust dans `rust-toolchain.toml`. Cargo et NuGet fournissent le compilateur Protobuf. [Prérequis Tauri](https://v2.tauri.app/start/prerequisites/)

Installez aussi Python 3.13 et Ollama, puis préparez un modèle local qui gère les appels d’outils (validé avec `qwen3:8b`) et, si vous voulez que l’agent puisse décrire une fenêtre, un modèle de vision local (par exemple `gemma3:4b`). Depuis la racine du projet, dans PowerShell :

```powershell
npm --prefix apps/desktop ci
$env:AIWIN11_PYTHON = (Get-Command python.exe -CommandType Application | Select-Object -First 1).Source
dotnet publish services/windows/AiWin11.Windows.csproj --configuration Debug --runtime win-x64 --self-contained false --output target/debug
dotnet publish services/admin/AiWin11.Admin.csproj --configuration Debug --runtime win-x64 --self-contained false --output target/debug
Copy-Item -LiteralPath services/agent/agent.py -Destination target/debug/aiwin11_agent.py
npm --prefix apps/desktop run tauri -- dev
```

Si vous utilisez Ollama portable depuis `.tools/ollama`, lancez-le dans un terminal séparé, en mode local strict, et gardez ce terminal ouvert :

```powershell
$env:OLLAMA_HOST = '127.0.0.1:11434'
$env:OLLAMA_NO_CLOUD = '1'
$env:OLLAMA_MODELS = Join-Path $PWD '.tools/ollama-models'
.\.tools\ollama\ollama.exe serve
```

La description de fenêtre et l’analyse d’image exigent qu’Ollama confirme que ses fonctions cloud sont désactivées ; le lancement ci-dessus le fait. [Documentation Ollama](https://docs.ollama.com/faq#how-do-i-disable-ollamas-cloud-features)

## Installer AiWin11 (sans environnement de développement)

Double-clique sur `Installateur.cmd` (ou `scripts\package.ps1`) : il produit `target\release\bundle\nsis\AiWin11_0.1.0_x64-setup.exe`. Cet installateur s’installe pour l’utilisateur courant, sans droits administrateur, dans `%LOCALAPPDATA%\AiWin11`, avec le pont Windows, l’assistant administrateur et l’agent. Il faut encore installer séparément Python 3.13 (dans le `PATH`), le runtime .NET 10 et Ollama. AiWin11 trouve Python tout seul (`AIWIN11_PYTHON` reste possible pour forcer un interpréteur). Installateur non signé : Windows SmartScreen peut demander une confirmation.

## Utiliser l’assistant

1. Dans **Modèles**, détectez les modèles Ollama et choisissez le modèle de l’agent. Facultatif : détectez les modèles de vision et choisissez-en un. Ces choix (fournisseur, modèle, modèle de vision, identifiant du modèle OpenAI) sont mémorisés et revérifiés auprès d’Ollama au démarrage suivant.
2. Dans **Sécurité**, choisissez le profil d’autonomie (voir plus bas). **Assisté** est le profil par défaut.
3. Dans **Accueil**, écrivez une demande en langage naturel (2 000 caractères au maximum).

La réponse s’affiche au fil de sa génération. L’agent peut enchaîner plusieurs actions, jusqu’à 8 étapes. À la fin, AiWin11 affiche sa réponse et, en dessous, **les actions réellement effectuées** : ce rapport est écrit par AiWin11 à partir de ce qui s’est vraiment passé, pas par le modèle.

Ce que l’agent peut faire :

| Exemple de demande | Action |
|---|---|
| « Ouvre le Bloc-notes », « Ouvre Paint », « Ouvre la Table des caractères » | Ouvre une nouvelle fenêtre de l’application (liste fermée) et la vérifie |
| « Ouvre le Bloc-notes et écris-y ma liste de courses : pain, lait » | Écrit dans le Bloc-notes qu’il vient d’ouvrir, seulement si le document est vide, puis relit le texte |
| « Résume notes.txt », « Quels fichiers ai-je ? » | Liste et lit les fichiers `.txt` du dossier affiché dans **Fichiers** |
| « Copie notes.txt vers sauvegarde.txt », « Renomme brouillon.txt en idees.txt » | Copie ou renomme dans ce dossier, sans jamais remplacer un fichier existant |
| « Mets ancien.txt à la corbeille » | Déplace le fichier dans la corbeille d’AiWin11 (`%LOCALAPPDATA%\app.aiwin11.desktop\trash`) : rien n’est supprimé, il reste récupérable |
| « Mon PC est lent, regarde ce qui se passe » | Lit en lecture seule la mémoire, la charge du processeur, l’espace des disques et, si besoin, les processus ; ne modifie rien |
| « Quelles fenêtres sont ouvertes ? », « Réduis la fenêtre du navigateur » | Liste les fenêtres (après ton accord) puis met au premier plan, réduit, restaure ou agrandit l’une d’elles ; ne ferme jamais de fenêtre |
| « Qu’est-ce que j’ai copié ? », « Copie cette adresse dans le presse-papiers » | Lit ou remplace le texte du presse-papiers, toujours après ton accord (l’ancien contenu est perdu en cas d’écriture) |
| « Y a-t-il des services Windows arrêtés ? » | Liste en lecture seule les services automatiques arrêtés puis des services en cours ; ne démarre ni n’arrête rien |
| « Quels programmes utilisent le plus de mémoire ? » | Liste en lecture seule les 15 processus qui utilisent le plus de mémoire (nom, mémoire, temps processeur) ; rien n’est arrêté |
| « Que montre la fenêtre que j’utilisais ? » | Lit la structure (types et noms des contrôles, jamais les champs de mot de passe) de la fenêtre utilisée juste avant AiWin11 |
| « Décris la fenêtre que j’utilisais » | Fait décrire cette fenêtre par le modèle de vision **local** ; l’agent ne reçoit que la description |

Quand une action demande ton accord, un encadré la décrit exactement (fichiers, nombre de caractères, texte complet à écrire, titre de la fenêtre…). Choisis **Autoriser** ou **Refuser** ; le focus clavier commence sur **Refuser**. Sans réponse sous 60 secondes, la proposition expire. **Annuler** arrête la génération en cours ou une demande en attente d’accord ; une action déjà en cours d’exécution n’est plus annulable.

### Profils de sécurité

| Action | Observation | Assisté | Autonome limité | Avancé |
|---|---|---|---|---|
| Lister, lire des fichiers | Autorisé | Autorisé | Autorisé | Autorisé |
| Lire ou décrire une fenêtre | Demande | Demande | Demande | Demande |
| Ouvrir le Bloc-notes, copier, écrire dans le Bloc-notes | Refusé | Demande | Autorisé | Autorisé |
| Renommer | Refusé | Demande | Demande | Autorisé |
| Mettre à la corbeille | Refusé | Demande | Demande | Demande |

Le **mode simulation** remplace toute action avec effet par une simulation : l’agent te dit ce qu’il aurait fait, rien n’est modifié. Le profil s’applique à la demande suivante. Avec OpenAI, seuls l’ouverture du Bloc-notes et les réponses texte sont possibles : aucun contenu de fichier, de fenêtre ou d’image n’est envoyé au cloud.

Le bouton **Stoppe toutes les tâches**, toujours visible dans la barre latérale, arrête tout : une demande en réflexion ou en attente d’accord est annulée immédiatement (« Arrêt confirmé ») ; si une action est en cours d’exécution, elle n’est pas coupée au milieu (« Arrêt demandé ») et la demande s’arrête juste après, sans aucune autre action.

Toujours dans **Sécurité**, **Autorisations temporaires** permet de laisser une action (par exemple copier un fichier) se faire sans demander pendant 15 minutes à 2 heures. Cela ne vaut que là où ton profil demande ton accord, jamais pour la corbeille ni la lecture de l’écran ; **Révoquer** l’arrête dès l’action suivante.

La page **Sécurité** montre aussi la fiche de chaque outil : niveau de risque, possibilité d’annuler, simulation, usage local uniquement et délai maximal. Un outil **critique** demande toujours ton accord.

### Modèles par tâche et secours cloud

**Modèles** affiche le modèle de chaque tâche et les modèles chargés par Ollama (mémoire GPU). L’option **Local d’abord, OpenAI en secours** permet, si Ollama ne répond pas, de continuer la demande avec OpenAI : seulement si tu coches l’envoi pour cette demande, avant toute action, et jamais en mode privé.

### Mode privé, zone de notification et raccourci

**Sécurité → Mode privé** bloque toute requête vers un fournisseur cloud, même avec ton accord. AiWin11 a une icône dans la zone de notification (Ouvrir, Stoppe toutes les tâches, Quitter) et **Ctrl+Alt+Espace** le ramène au premier plan depuis n’importe quelle application, directement sur le champ de demande (palette de commandes).

### Opérations administrateur (désactivées par défaut)

AiWin11 tourne sans droits administrateur. Dans **Sécurité → Opérations administrateur**, tu peux autoriser l’agent à proposer une liste fermée d’opérations : démarrer, arrêter, redémarrer un service ou changer son démarrage (automatique, manuel, désactivé), et vider les fichiers temporaires de Windows de plus d’un jour. Chaque opération demande ton accord dans AiWin11, puis la confirmation Windows (UAC). Elle est exécutée par un programme séparé, `AiWin11.Admin.exe`, qui fait une seule opération puis s’arrête. Les services de sécurité (Defender, pare-feu), de mise à jour et les services vitaux de Windows sont toujours refusés.

### Fournisseur cloud optionnel (OpenAI ou DeepSeek)

Dans **Modèles**, choisis **Cloud**, puis le **Fournisseur cloud** (OpenAI ou DeepSeek), saisis l’identifiant du modèle (par exemple `deepseek-chat`) et enregistre la clé API : elle est conservée par Windows (`AiWin11/DeepSeek`), jamais dans la base ni dans un fichier.


Sélectionnez **OpenAI · Cloud** dans **Modèles**, renseignez un identifiant de modèle compatible avec les appels d’outils et enregistrez une clé API. Windows conserve la clé dans le Gestionnaire d’identifiants sous `AiWin11/OpenAI`. Dans **Accueil**, cochez l’envoi du texte de la demande à OpenAI, pour cette demande seulement : cet accord n’est jamais mémorisé. Le fournisseur choisi est mémorisé ; Ollama est sélectionné au premier démarrage. [Référence de l’API OpenAI](https://developers.openai.com/api/reference/resources/chat)

### Journal et reprise

**Activité** liste les demandes récentes avec leur état, et le journal de la session avec le code de chaque événement (décision de la politique, règle appliquée, résultat). Si AiWin11 s’arrête pendant une demande, celle-ci apparaît au démarrage suivant comme **interrompue** ; rien n’est relancé automatiquement. Si elle était en train d’exécuter une action, vérifie son résultat. Le journal est enregistré dans `%LOCALAPPDATA%\app.aiwin11.desktop\aiwin11.sqlite3`.

Dans **Activité**, la section **Historique et journal d’audit** règle la durée de conservation des sessions passées (90 jours par défaut ; les plus anciennes sont supprimées au démarrage), exporte le journal en JSON dans `%LOCALAPPDATA%\app.aiwin11.desktop\exports` et efface l’historique des sessions passées après confirmation (irréversible).

### Demandes enregistrées

Sous le champ de demande, **Enregistrer la demande** garde le texte (chiffré) ; un clic sur une demande enregistrée la remet dans le champ, tu n’as plus qu’à l’envoyer.

### Mémoire

Dis à l’agent « Souviens-toi que… » : après ton accord, l’information est enregistrée, chiffrée pour ton compte Windows. Il peut la relire dans les demandes suivantes (modèles locaux seulement). La page **Mémoire** liste et efface ces informations.

### Pages Inspecteur et Fichiers

Dans **Fichiers**, place un fichier `.txt` UTF-8 de 64 Kio maximum dans le dossier affiché, saisis son nom et clique sur **Afficher l’aperçu**, ou copie-le avec **Copier le fichier**.

Dans **Inspecteur**, **Inspecter une fenêtre** et **Capturer la fenêtre** lisent la fenêtre au premier plan après un compte à rebours de trois secondes. L’aperçu de capture reste en mémoire et peut être analysé avec un modèle de vision local. Pour tester la saisie contrôlée, ouvre la Table des caractères, choisis un chiffre, clique sur **Autoriser et saisir le chiffre** et remets la Table des caractères au premier plan dans les trois secondes.

## Vérifier le projet

Le plus simple : `powershell -ExecutionPolicy Bypass -File scripts/verify.ps1` lance tout (tests de l’agent, ruff et mypy, ESLint et Prettier, build de l’interface, `cargo fmt --check`, pont .NET, tests Rust, clippy) et affiche un résumé ; `-Live` ajoute les tests réels, `-Tauri` le build release.

Outils d’analyse de l’agent Python (une fois) :

```powershell
python -m venv .venv
.venv\Scripts\python -m pip install -r services/agent/requirements-dev.txt
```

Commandes individuelles :

```powershell
npm --prefix apps/desktop run lint
.venv\Scripts\python -m ruff check services/agent
.venv\Scripts\python -m mypy --config-file services/agent/pyproject.toml services/agent
cargo fmt --all --check
npm --prefix apps/desktop run build
python -B services/agent/test_agent.py
cargo test --locked -p aiwin11-core
cargo clippy --locked --workspace --all-targets -- -D warnings
dotnet build services/windows/AiWin11.Windows.csproj --configuration Release
npm --prefix apps/desktop run tauri -- build --no-bundle
```

`cargo test` lance aussi le vrai agent Python : `python` doit être accessible dans le `PATH`. Le workflow GitHub Actions utilise Windows Server 2025 pour compiler et tester ; la validation du bureau et des actions Windows exige une session interactive sur Windows 11 x64.

### Mesures de performance

```powershell
powershell -ExecutionPolicy Bypass -File scripts/measure.ps1 -Runs 3
cargo test --locked -p aiwin11-core --release measure_ -- --ignored --nocapture
```

Le script démarre l’application release (à construire avant), mesure le temps jusqu’à la création de la fenêtre et la mémoire au repos de l’application et des processus qu’elle a lancés, puis ferme uniquement l’instance qu’il a démarrée ; il refuse de s’exécuter si AiWin11 est déjà ouvert. Le test mesure la latence d’une décision de permission. Résultats de référence dans `docs/ROADMAP.md` §4.

### Tests avec un modèle réel

Les tests marqués `ignored` utilisent Ollama, le pont et l’agent publiés à côté de l’exécutable de test. Ils couvrent : refus puis ouverture approuvée du Bloc-notes ; ouverture puis écriture dans le Bloc-notes ; réponse texte sans action ; lecture de fichiers ; consigne piégée dans un fichier ; copie refusée puis approuvée ; renommage puis mise à la corbeille ; lecture et description de la Table des caractères ouverte par le test ; démarrage du pont de réserve.

```powershell
$env:AIWIN11_TEST_MODEL = 'qwen3:8b'
$env:AIWIN11_TEST_VISION_MODEL = 'gemma3:4b'
$env:AIWIN11_PYTHON = (Get-Command python.exe -CommandType Application | Select-Object -First 1).Source
dotnet publish services/windows/AiWin11.Windows.csproj --configuration Debug --runtime win-x64 --self-contained false --output target/debug/deps
dotnet publish services/admin/AiWin11.Admin.csproj --configuration Debug --runtime win-x64 --self-contained false --output target/debug/deps
Copy-Item -LiteralPath services/agent/agent.py -Destination target/debug/deps/aiwin11_agent.py
cargo test --locked -p aiwin11-core -- --ignored --test-threads=1
```

Fermez le Bloc-notes avant ces tests : s’il est déjà ouvert, il peut ouvrir la demande dans un onglet existant et la nouvelle fenêtre ne peut pas être vérifiée. Les très petits modèles peuvent appeler un outil avec des arguments mal formés (`qwen3:0.6b` envoie par exemple `{"application":{"type":"notepad"}}`) : l’agent rejette la proposition (`AGENT_REJECTED`) et rien n’est exécuté.

## Dépannage

- **Ollama indisponible** : vérifiez que son API répond sur `127.0.0.1:11434`.
- **Fenêtre du Bloc-notes non confirmée** : le Bloc-notes a probablement réutilisé une fenêtre ou un onglet existant.
- **Écriture refusée (`NOTEPAD_NOT_EMPTY`)** : le Bloc-notes a restauré un onglet qui contient déjà du texte ; AiWin11 n’écrit jamais par-dessus.
- **Premier démarrage lent** : au lancement, AiWin11 démarre en arrière-plan un processus `AiWin11.Windows.exe` de réserve, qui attend sans rien faire la prochaine commande autorisée. Le premier démarrage du pont après une installation peut prendre une vingtaine de secondes (chargement de .NET, analyse antivirus) sans retarder les actions. Ce processus n’écoute aucun port et se termine avec l’application.
- **Port 1420 occupé** : arrêtez le serveur de développement qui l’utilise.
- **Édition de liens en échec** : vérifiez les outils C++ x64 et le SDK Windows dans Visual Studio Installer. **Fenêtre blanche** : vérifiez que WebView2 est installé.
- Une base créée par une version plus récente de l’application nécessite cette version pour s’ouvrir.

## Périmètre

Le cœur s’exécute dans le processus Tauri ; le pont utilise le runtime .NET 10 x64 et l’agent Python tourne dans un processus séparé sans accès réseau. Le transport gRPC entre processus séparés, le routage automatique entre modèles, la mémoire, MCP, la voix et les workflows ne sont pas encore implémentés. Voir [la feuille de route](docs/ROADMAP.md), [la spécification](docs/SPECIFICATION.md), [les contrats](docs/CONTRACTS.md) et [le backlog](docs/BACKLOG.md).
