# AiWin11

Application personnelle pour Windows 11 x64. Le premier scénario relie une interface Tauri/React, un modèle Ollama, un agent Python et un pont .NET. Le broker Rust applique la politique d’exécution et écrit dans le journal SQLite.

## Démarrer sous Windows 11

Installez Node.js 24 avec npm, Rustup avec la cible MSVC x64, les outils Visual Studio C++ (charge de travail « Développement Desktop en C++ »), WebView2 et le SDK .NET 10 pour le pont Windows. Rustup lit la version de Rust dans `rust-toolchain.toml`. Cargo et NuGet fournissent le compilateur Protobuf. [Prérequis Tauri](https://v2.tauri.app/start/prerequisites/)

Installez aussi Python 3.13 et Ollama, puis préparez un modèle local qui annonce la prise en charge des appels d’outils. Depuis la racine du projet, dans PowerShell :

```powershell
npm --prefix apps/desktop ci
$env:AIWIN11_PYTHON = (Get-Command python.exe -CommandType Application | Select-Object -First 1).Source
dotnet publish services/windows/AiWin11.Windows.csproj --configuration Debug --runtime win-x64 --self-contained false --output target/debug
Copy-Item -LiteralPath services/agent/agent.py -Destination target/debug/aiwin11_agent.py
npm --prefix apps/desktop run tauri -- dev
```

Dans **Modèles**, détectez les modèles Ollama et sélectionnez-en un. Dans **Accueil**, saisissez une demande en langage naturel (2 000 caractères au maximum), par exemple « Peux-tu ouvrir le Bloc-notes ? ». Pour que l’agent puisse réellement ouvrir le Bloc-notes, cochez l’autorisation pour cette demande ; sans elle, le broker refuse l’ouverture. Si la demande ne correspond à aucun outil disponible, l’agent répond en texte, sans aucune action sur Windows. Le résultat apparaît dans le chat, avec les événements de la session dans **Activité**. Le journal est enregistré dans `%LOCALAPPDATA%\app.aiwin11.desktop\aiwin11.sqlite3`. Si Ollama est indisponible, vérifiez que son API répond sur `127.0.0.1:11434`. Si une fenêtre n’est pas vérifiée, vérifiez si le Bloc-notes a réutilisé un onglet ou une fenêtre existante.

Si vous utilisez Ollama portable depuis `.tools/ollama`, ouvrez un terminal PowerShell dans le projet et lancez-le avec le dossier de modèles du projet. Gardez ce terminal ouvert pendant l’utilisation d’AiWin11.

```powershell
$env:OLLAMA_HOST = '127.0.0.1:11434'
$env:OLLAMA_NO_CLOUD = '1'
$env:OLLAMA_MODELS = Join-Path $PWD '.tools/ollama-models'
.\.tools\ollama\ollama.exe serve
```

Pour utiliser le fournisseur cloud optionnel, sélectionnez **OpenAI · Cloud** dans **Modèles**, renseignez un identifiant de modèle compatible avec les appels d’outils et enregistrez une clé API. Windows conserve la clé dans le Gestionnaire d’identifiants sous `AiWin11/OpenAI`. Dans **Accueil**, autorisez séparément l’envoi du texte de votre demande à OpenAI et l’ouverture du Bloc-notes. Ollama est sélectionné au démarrage et les deux fournisseurs sont sélectionnés manuellement. [Référence de l’API OpenAI](https://developers.openai.com/api/reference/resources/chat)

## Vérifier le projet

Dans **Fichiers**, place un fichier `.txt` UTF-8 de 64 Kio maximum dans le dossier affiché. Saisis son nom et clique sur **Afficher l’aperçu**. AiWin11 lit le fichier localement et journalise le résultat de la lecture. L’aperçu disparaît quand tu changes de page ou cliques sur **Effacer l’aperçu**.

Pour le dupliquer dans ce dossier, renseigne **Nom de la copie** et clique sur **Copier le fichier**. Si ce nom existe déjà, choisis-en un autre. L’activité affiche le résultat de l’opération.

Dans **Inspecteur**, clique sur **Inspecter une fenêtre**, puis passe à la fenêtre à examiner dans les trois secondes. Reviens dans AiWin11 pour lire l’arborescence des contrôles, leur état et le focus. Le parcours s’arrête après 40 contrôles ou six niveaux sous la racine. Le journal indique si l’inspection a abouti. Le pont utilise le runtime **.NET Desktop 10 x64**.

**Capturer la fenêtre** utilise le même compte à rebours. L’aperçu reste en mémoire dans AiWin11. Tu peux l’effacer avec **Effacer l’aperçu** ou en quittant la page. Le journal enregistre l’issue de la capture. Certaines fenêtres protégées ou incompatibles avec la capture Windows peuvent renvoyer une erreur.

Pour obtenir une description, détecte les modèles de vision depuis l’aperçu, choisis un modèle installé, puis clique sur **Analyser avec Ollama**. Cette fonction exige une version d’Ollama qui confirme son mode sans cloud. Le lancement portable ci-dessus configure ce mode. [Documentation Ollama](https://docs.ollama.com/faq#how-do-i-disable-ollamas-cloud-features)

Pour tester la saisie contrôlée, ouvre la Table des caractères Windows. Dans **Inspecteur**, choisis un chiffre et clique sur **Autoriser et saisir le chiffre**, puis remets la Table des caractères au premier plan dans les trois secondes. Le chiffre remplace le contenu du champ **Caractères à copier**. AiWin11 relit la valeur pour confirmer le résultat.

```powershell
npm --prefix apps/desktop run build
python -B services/agent/test_agent.py
cargo test --locked -p aiwin11-core
cargo clippy --locked --workspace --all-targets -- -D warnings
dotnet build services/windows/AiWin11.Windows.csproj --configuration Release
npm --prefix apps/desktop run tauri -- build --no-bundle
```

Le workflow GitHub Actions utilise Windows Server 2025 pour compiler et tester. La validation du bureau et des actions Windows exige une session interactive sur Windows 11 x64.

`cargo test` lance aussi le vrai agent Python : `python` doit être accessible dans le `PATH`.

Pour exécuter les tests d’intégration avec un modèle réel, démarrez Ollama et choisissez un modèle local compatible avec les appels d’outils. Le premier test vérifie un refus du broker, puis une ouverture autorisée et journalisée. Le second vérifie qu’une question sans outil applicable reçoit une réponse texte sans aucune action. Publiez les composants à côté de l’exécutable de test dans `target/debug/deps`.

```powershell
$env:AIWIN11_TEST_MODEL = Read-Host 'Identifiant du modèle local'
$env:AIWIN11_PYTHON = (Get-Command python.exe -CommandType Application | Select-Object -First 1).Source
dotnet publish services/windows/AiWin11.Windows.csproj --configuration Debug --runtime win-x64 --self-contained false --output target/debug/deps
Copy-Item -LiteralPath services/agent/agent.py -Destination target/debug/deps/aiwin11_agent.py
cargo test --locked -p aiwin11-core task::tests -- --ignored
```

Si le port 1420 est occupé, arrêtez le serveur de développement qui l’utilise avant de relancer. Si l’édition de liens échoue, vérifiez les outils C++ x64 et le SDK Windows dans Visual Studio Installer. Si la fenêtre ne se rend pas, vérifiez que WebView2 est installé. Une base créée par une version plus récente de l’application nécessite cette version de l’application pour s’ouvrir.

## Périmètre

Le cœur s’exécute dans Tauri. Le pont utilise le runtime .NET 10 x64. Le chat accepte des demandes libres : l’agent choisit entre une réponse texte et un outil de son registre, qui ne contient pour l’instant que l’ouverture du Bloc-notes. Une action nécessite une autorisation valable pour la demande en cours. L’annulation empêche l’exécution tant que le broker n’a pas commencé à traiter l’action. L’appel au fournisseur en cours peut continuer jusqu’à sa réponse ou son délai d’expiration. Les événements sont actualisés pendant la demande. La génération conversationnelle et le flux de jetons restent à intégrer. Voir [la spécification](docs/SPECIFICATION.md), [les contrats](docs/CONTRACTS.md) et [le backlog](docs/BACKLOG.md).
