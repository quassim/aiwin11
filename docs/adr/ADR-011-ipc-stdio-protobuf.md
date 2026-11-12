# ADR-011 — Communication interprocessus : Protobuf sur stdio plutôt que gRPC (pour l’instant)

**Statut** : accepté le 12 novembre 2026 (remplace provisoirement la partie transport d’ADR-007).

## Contexte

ADR-007 prévoyait gRPC sur des canaux nommés Windows entre le cœur, le broker, le pont .NET, l’agent Python et la passerelle de modèles, avec six processus logiques. L’implémentation actuelle :

- le cœur, le broker, le moteur de politiques et la passerelle de modèles tournent dans le processus Tauri (Rust) ;
- le pont Windows (.NET), l’agent Python, l’assistant élevé et les serveurs MCP sont des **processus séparés**, lancés pour une commande puis arrêtés (un pont de réserve est démarré à l’avance) ;
- les messages vers le pont sont des `Command` / `Result` Protobuf versionnés (`contracts/v1/aiwin11.proto`), sur stdin/stdout, bornés en taille et en durée ;
- aucun port réseau n’est ouvert, aucun canal nommé n’est exposé.

## Décision

Garder ce modèle « un processus par commande, Protobuf sur stdio » tant qu’aucun service n’a besoin d’être permanent.

Raisons :

1. **Surface d’attaque minimale** : pas de canal nommé à protéger par ACL, pas d’authentification de pair à construire ; seul le cœur détient les tubes de ses enfants.
2. **Isolation des pannes déjà obtenue** : un plantage du pont, de l’agent ou d’un serveur MCP termine la commande par un code d’erreur ; le cœur et l’interface continuent (verrou du cœur rétabli après une panique).
3. **Contrat identique** : les messages sont ceux qu’un service gRPC exposerait ; passer à gRPC ne changera que le transport.
4. **Coût mesuré** : le démarrage à froid du pont est absorbé par le pont de réserve ; la décision de permission reste à p95 7 ms.

## Conséquences

- Le broker et le moteur de politiques partagent la mémoire du processus de l’interface : une faille de la WebView qui permettrait d’exécuter du code natif atteindrait aussi le broker. Les commandes Tauri restent la seule entrée (capacités déclarées une par une).
- Pas de service permanent : la planification ne tourne que lorsque AiWin11 est ouvert.

## Quand revenir à ADR-007

Passer à des services séparés (gRPC sur canaux nommés avec ACL par utilisateur) quand l’un de ces besoins apparaît : un agent permanent en arrière-plan sans fenêtre, plusieurs clients (overlay séparé, autre application), ou l’isolation du broker dans un processus distinct de la WebView.
