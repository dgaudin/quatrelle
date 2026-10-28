# Ce fork : dgaudin/quatrelle

Ce dépôt est un fork de [ZIDAN44/mailcraft](https://github.com/ZIDAN44/mailcraft), renommé
**Quatrelle** (son nom d'origine était MailCraft). Il s'en écarte
volontairement sur l'apparence, pour ressembler davantage à Outlook et s'intégrer au bureau, et il
ajoute des fonctions Exchange. Ce fichier dit **quoi**, **pourquoi**, et **comment revenir en
arrière** si besoin.

Point de départ : le commit `8cbd116` (« Docs: v0.15 »). Les modifications sont sur la branche
`claude/practical-gauss-g3t65c`.

## 1. Les icônes : Fluent UI System Icons de Microsoft

**L'écart.** Le projet d'origine interdit toute illustration Microsoft : ses icônes sont des tracés
dessinés dans le code. Ce fork utilise à la place les **Fluent UI System Icons** de Microsoft, que
ce soit pour le ruban, les modules ou les menus.

**Pourquoi.** On voulait des icônes libres de droit, bien connues des projets web, et proches de
celles d'Outlook. Fluent est la famille d'icônes du Outlook actuel, et Microsoft la publie sous
licence **MIT** ([microsoft/fluentui-system-icons](https://github.com/microsoft/fluentui-system-icons)).
On peut donc l'utiliser, la modifier et la redistribuer, à condition de garder la licence.

**Comment.**
- Les fichiers sont dans `assets/icons/fluent/` : les contours « regular » de 20 pixels, copiés
  **sans modification** depuis le paquet npm `@fluentui/svg-icons` 1.1.292. La licence MIT est
  dans `assets/icons/fluent/LICENSE.txt`.
- Chaque fichier a sa ligne dans `ATTRIBUTION.md`. C'est exigé par `cargo xtask assets`, qui
  bloque le CI sinon.
- `crates/ui-egui/src/icons.rs` associe chaque icône à son fichier. Le dessin se fait avec
  `resvg`, une bibliothèque en Rust pur qui fonctionne aussi dans le navigateur. L'icône est
  rendue une fois par taille, puis colorée au moment de l'affichage.
- Les anciens tracés en code restent en secours, si un SVG ne peut pas être rendu.
- `CLAUDE.md`, `AGENTS.md` et `README.md` mentionnent cette exception comme une décision du
  propriétaire.

**Les limites, qui restent valables.**
- Aucune icône « classique » d'Office, ni aucune image, aucun son, aucun modèle ou texte d'Outlook.
- L'icône de l'application et sa couleur restent celles de Quatrelle.
- Une nouvelle icône doit être un autre fichier Fluent, ajouté de la même façon : fichier
  inchangé, plus une ligne dans `ATTRIBUTION.md`.

**Revenir en arrière.** Annuler le commit `69bef61`, ainsi que les ajouts d'icônes des commits qui
le suivent. Les tracés en code reprennent alors automatiquement.

## 2. La fenêtre suit le bureau

**L'écart.** Le projet d'origine dessinait sa propre fenêtre : une barre grise sans titre sous
GNOME Wayland, sa couleur sarcelle partout, sa police, et un mode clair forcé sous Linux.

**Pourquoi.** Une application de bureau doit avoir les mêmes boutons de fenêtre (réduire,
agrandir, fermer), les mêmes couleurs et les mêmes polices que les autres applications.

**Comment** (commit `3f5a21d`, code dans `apps/quatrelle/src/appearance.rs`).
- **Barre de titre.** Là où le gestionnaire de fenêtres la dessine (KDE, wlroots, X11, Windows,
  macOS), c'est lui qui le fait. Sous GNOME Wayland, la fonction `wayland-csd-adwaita` de `winit`
  en dessine une identique à celle de GNOME : titre, boutons placés selon le réglage
  `button-layout`, clair ou sombre.
- **Clair ou sombre.** Sous Linux, le réglage est lu via le portail XDG (GNOME et KDE). Avant,
  « Comme le système » donnait toujours le mode clair.
- **Couleur d'accent.**
  - Elle vient du système : DWM sous Windows, l'accent de macOS, `accent-color` du portail sous
    Linux. Elle remplace le sarcelle de Quatrelle.
  - Elle est éclaircie ou foncée jusqu'à rester lisible sur les volets.
  - **Option, activée par défaut** : Options › « Utiliser la couleur d'accentuation du système ».
    Décochez-la pour retrouver le sarcelle.
- **Police.**
  - Sous Linux, c'est celle de l'interface du bureau : `font-name` de GNOME, `kdeglobals` de KDE,
    sinon le `sans-serif` de fontconfig.
  - Elle n'est prise que si elle a un gras.
  - Aucune police n'est embarquée.
- **Mise à jour.** Le style est relu toutes les quelques secondes : un changement dans les
  réglages du bureau apparaît sans redémarrer.

## 3. L'apparence d'Outlook

Le modèle est le Outlook classique, **observé de l'extérieur** à partir de captures d'écran de sa
fenêtre. Ces captures ne sont pas dans le dépôt, et rien n'a été lu dans l'installation d'Outlook.
Seules la disposition et les noms des fonctions sont repris.

| Élément | Ce qui change | Commit | Option pour revenir |
|---|---|---|---|
| Barre du haut | Neutre (couleur des volets) au lieu d'être remplie de couleur d'accent ; recherche centrée | `2d678e2` | — |
| Ruban | Grands boutons avec leur nom dessous, rangés en groupes nommés | `3a89df5` | — |
| Ruban repliable | Se replie sur ses onglets avec ⌃ ou Ctrl+F1 ; état gardé d'un lancement à l'autre | `a55c4e9` | le ⌃ / Ctrl+F1 |
| Modules | Rail vertical à gauche (Courrier, Calendrier, Contacts…) | `7c06664` | — |
| Liste des messages | Tableau à colonnes triables quand la liste est large | `0e537a4` | Affichage › Liste des messages : Automatique / Cartes / Tableau |
| Volet des dossiers | Noms seuls, sans icône devant, lignes plus serrées | `a55c4e9` | — |
| Barre To-Do | À droite du courrier : le mois en cours et les rendez-vous de la semaine | `0237901` | Affichage › Barre des tâches |
| Calendrier | Vue Semaine de travail, en-têtes de jours, deux mois dans le volet, groupes de calendriers | `2c30e94` | — |

**Pourquoi « Automatique » pour la liste.** Outlook montre des cartes quand le volet de lecture est
à droite (la liste est étroite), et un tableau quand il est en bas ou masqué. Le mode Automatique
fait la même chose ; les deux autres choix forcent l'un ou l'autre.

## 4. Fonctions Exchange ajoutées

Toutes s'appuient sur les spécifications publiques de Microsoft (EWS, Graph, [MS-OXOMSG],
[MS-OXOCFG]), pas sur le code d'Outlook.

- **Boutons de vote** (`1fb5aaf`)
  - Les messages avec des boutons de vote les affichent dans le volet de lecture.
  - La réponse est envoyée comme Outlook l'attend (`PidLidVerbResponse`), pour que l'expéditeur la
    voie comptée.
  - Fonctionne sur Exchange (EWS) et sur Microsoft 365 (Graph).
- **Boîtes partagées et délégations** (`27e3dbe`)
  - Fichier › Ouvrir une autre boîte aux lettres.
  - La boîte s'affiche sous un dossier portant son adresse, avec son calendrier.
  - Un message supprimé va dans ses propres Éléments supprimés.
- **Groupes de calendriers**
  - Les groupes du volet Calendrier d'Outlook (« Équipe : … », « Calendriers partagés »,
    salles) sont repris avec les calendriers des collègues qu'ils contiennent.
  - Sur Microsoft 365 (`54bba09`).
  - Sur Exchange **on-premise** (`0817f83`), à partir des raccourcis de navigation qu'Outlook
    enregistre dans la boîte, décrits dans [MS-OXOCFG] §2.2.9.
- **Glisser-déposer dans le calendrier**
  - Un rendez-vous se déplace à la souris dans les vues Jour, Semaine de travail, Semaine et
    Mois (à la minute près, par quart d'heure) ; son bord inférieur change l'heure de fin ;
    Échap abandonne ; Ctrl+Z remet en place.
  - Une occurrence d'une série peut être déplacée seule (une instance avec `RECURRENCE-ID`) :
    CalDAV l'écrit dans le fichier de la série ; Exchange (EWS) demande l'élément de
    l'occurrence (`OccurrenceItemId`) et le modifie, puis relit avec la série ses occurrences
    modifiées (`ModifiedOccurrences`) et supprimées (`DeletedOccurrences`). Sur Microsoft 365
    (Graph), seule la série entière se déplace : Graph ne relit pas les exceptions.
  - Pour une réunion organisée par l'utilisateur, la question d'Outlook : « Envoyer la mise à
    jour / Enregistrer sans envoyer ». Sur Exchange, c'est le serveur qui envoie la mise à jour
    (`SendToAllAndSaveCopy`, sur la clé de modification de l'élément), aux invités qu'il connaît
    désormais (`RequiredAttendees`) ; plus d'invitation iMIP en double. Sur Microsoft 365, le
    serveur prévient toujours. Ailleurs (CalDAV, calendrier local), un `REQUEST` par courrier,
    `SEQUENCE` incrémenté.
- **Rappels**
  - La notification de bureau d'un rappel reste jusqu'à réponse, sonne, et a les boutons
    « Répéter dans 5 minutes » et « Ignorer » (Linux, via `notify-rust`) ; un clic dessus ramène
    la fenêtre.
  - Option « Afficher les rappels au-dessus des autres fenêtres » (activée par défaut) : la
    fenêtre Rappels s'ouvre dans une fenêtre à part, toujours au premier plan, même Quatrelle
    réduit (`RequestPaintWhileHidden` tant qu'un rappel est dû).

### Exchange Online : inscription d'application propre, boîtes partagées, détection

Trois ajouts (2026-10-22) pour Microsoft 365 (Graph), détaillés dans `docs/oauth.md` :

- **Inscription d'application de l'organisation.** L'identifiant d'application (client) ne venait
  que de `QUATRELLE_MICROSOFT_CLIENT_ID` (à la compilation ou dans l'environnement), et le
  locataire était figé à `common`. Les deux se saisissent désormais **par compte**, dans Ajouter
  un compte › Paramètres du serveur › « Connexion Microsoft : l'inscription d'application de votre
  organisation », et dans les paramètres du compte ensuite. Ils sont gardés dans `accounts.json`
  (`oauth_client_id`, `oauth_tenant`, sans secret) et remplacent l'identifiant intégré et
  `common` pour ce compte. Une organisation peut ainsi inscrire Quatrelle elle-même, en
  locataire unique, et faire approuver l'application par son administrateur ; une copie de
  Quatrelle compilée sans aucune inscription ajoute quand même un tel compte. En ligne de
  commande : `oauthClientId` et `oauthTenant` sur `account.add` et `account.update`.
- **Boîtes partagées.** Les permissions Graph demandées comprennent `Mail.ReadWrite.Shared`,
  `Mail.Send.Shared` et `Calendars.ReadWrite.Shared`, pour que les boîtes et calendriers
  partagés s'ouvrent. Une permission ajoutée oblige Microsoft à redemander le consentement à la
  prochaine connexion : le navigateur s'ouvre une fois de plus, rien n'est perdu.
- **Détection de Microsoft 365.** Quand l'enregistrement MX du domaine est celui d'Exchange
  Online (`<domaine>.mail.protection.outlook.com`), ou que son enregistrement
  `_autodiscover._tcp` pointe vers Microsoft, le dialogue d'ajout propose **Exchange
  (Microsoft 365)** par Graph plutôt qu'IMAP, souvent désactivé dans ces organisations. Les
  réglages IMAP trouvés par ailleurs restent sous Paramètres du serveur.

## 5. Construire un paquet (.rpm, .deb, AppImage…)

- **Sur GitHub.** Les Actions sont activées sur ce fork. Le workflow
  **Release (unsigned test build)** (`.github/workflows/release-wip.yml`) se lance à la main :
  Actions › Run workflow, en choisissant la branche. Il publie une pré-version
  `v<version>-test.<n>` avec tous les paquets, non signés.
- **En local.** Avec [nfpm](https://nfpm.goreleaser.com) installé :
  `packaging/linux/package.sh --formats rpm`. Le paquet sort dans `dist/release/`.

## 6. Ce qui ne change pas

- La règle « ne jamais planter » : pas de `unwrap()` ni de `panic!` hors des tests, et chaque
  correction de plantage vient avec son test.
- Les mots de passe restent dans le trousseau du système.
- Les vérifications de certificats restent toujours actives.
- Le contenu distant n'est jamais chargé sans action de l'utilisateur.
- Tout le texte de l'interface passe par `tl!` / `tf!`, traduit dans les 14 langues.
- `cargo xtask ci` doit rester vert avant chaque commit.
- Les identifiants d'application (`ai.storyteller.quatrelle`), les noms des exécutables et le
  `UpgradeCode` de l'installeur Windows sont inchangés.

## Suivre le projet d'origine

Pour récupérer les nouveautés de ZIDAN44/mailcraft, la méthode qui a servi pour la v0.22 (voir
plus bas) est la plus sûre : une fusion directe bute sur le renommage MailCraft → Quatrelle à
chaque ligne que l'origine touche.

```sh
git fetch https://github.com/ZIDAN44/mailcraft main && git branch -f upstream-main FETCH_HEAD
```

Les conflits les plus probables concernent les fichiers touchés par ce fork :
`crates/ui-egui/src/icons.rs`, `widgets.rs`, `lib.rs`, `list.rs`, `calendar.rs`, `ATTRIBUTION.md`
et `CLAUDE.md`. Gardez la version de ce fork pour les icônes et le style, et celle d'origine pour
le reste.

### État de la récupération : v0.17 intégrée (2026-10-19)

`upstream/main` a été fusionnée dans la branche de ce fork : 42 commits (v0.16, v0.17, egui 0.37,
Rust 1.95 minimum). Les deux versions n'ont eu aucun commit divergent — `main` n'avait rien de
propre — donc la fusion n'a rien réécrit. 19 fichiers ont demandé une résolution :

- **Traductions** (5 `.tsv`) : les deux côtés ajoutaient des clés différentes ; elles sont toutes
  gardées, sans doublon.
- **Cache des lignes** (`list.rs`) : le cache de l'origine (perf des grosses boîtes) est gardé, et
  sa clé inclut la hauteur du mode table de ce fork.
- **Filets de sécurité** : `Cargo.lock` et `ROADMAP.md` viennent de l'origine (le journal du fork y
  est ajouté) ; `logfile.rs` est la version de l'origine (rotation, tests).
- **Ajouts des deux côtés** : `widgets.rs` (vos widgets de ruban + `switch`/`segmented`), `ews.rs`,
  `ews_pim.rs`, `fake_ews.rs`, `fake_graph.rs` gardent les champs et fonctions des deux.
- **Envois Exchange** : la connexion rapide de l'origine (une deuxième connexion par compte)
  **remplace** le thread par message de ce fork. Même service — envoyer pendant une synchronisation —
  mais dans l'ordre, et les réponses automatiques en profitent. C'était la seule vraie divergence de
  conception ; trois tests en dépendent, ils passent.
- **Fusion non retenue** : la page File › Info garde la mise en page de l'origine (cartes et
  réglages), tandis qu'Open & Export garde les tuiles de ce fork. À harmoniser si souhaité.

Preuves de la fusion : `cargo xtask ci` vert (fmt, `clippy -D warnings`, 366 tests, layers, assets,
wasm), un build `--workspace` complet, et le test d'envoi Exchange relancé trois fois sans échec.

### État de la récupération : v0.18 et v0.19 intégrées (2026-10-19)

Second passage, sans recréer de remote : `git fetch https://github.com/ZIDAN44/mailcraft main` puis
fusion de `FETCH_HEAD` (10 commits, v0.18 + v0.19 — boîtes partagées Exchange, fiche contact,
sign-in SMTP propre, fichier `.quatrelle` conservé à la suppression d'un compte). Aucun commit de
ce fork n'a été réécrit ; 23 fichiers ont demandé une résolution.

- **Traductions** (14 `.tsv`) : union des deux côtés, sans doublon.
- **Modèle** (`model/lib.rs`) : le `Folder` prend les champs de l'origine `mailbox` et
  `mailbox_role` (une boîte partagée est une boîte à part entière). Le tri du volet unionne la
  règle de l'origine (Exchange, dossiers marqués) et celle de ce fork (Graph, dossiers nommés
  d'après l'adresse, détectés par le `@`).
- **Exchange (EWS)** : l'implémentation de l'origine **remplace** celle de ce fork — elle lit les
  boîtes partagées qu'Autodiscover associe au compte en plus de celles ajoutées à la main, avec les
  vrais rôles de dossiers. Le `address_of` de ce fork reste (il sert aux calendriers).
- **Graph** : ce fork garde son implémentation (boîtes partagées sous `/users/<adresse>`, que
  l'origine n'a pas), plus le `name_special` de l'origine.
- **Boîte du même rôle** (`session.rs`) : `folder()` et `folder_beside()` reconnaissent les dossiers
  d'une autre boîte des deux façons (`mailbox` de l'origine, préfixe d'adresse de ce fork), et
  `move_selection` groupe par boîte — un message supprimé dans une boîte partagée Exchange va bien
  dans *ses* Éléments supprimés. Un test le vérifie désormais sur les deux formes de nommage.
- **Dialogues et boîtes partagées** : unifiés sur l'entrée de ce fork (commit `5343945`) —
  « Ouvrir une autre boîte aux lettres… », plus riche (Graph et calendrier), garde ses commandes
  `account.openMailbox`/`account.closeMailbox` ; le dialogue et les commandes de l'origine
  (`AddSharedMailbox`, `account.addSharedMailbox`/`removeSharedMailbox`) sont retirés. Le menu d'une
  boîte partagée ferme désormais via `account.closeMailbox`, et `shared_mailboxes` se compare sans
  tenir compte de la casse (une entrée laissée par l'ancienne voie restait sinon impossible à fermer).
- **Doublure de test** (`fake_ews.rs`) : celle de l'origine (boîtes partagées, ancrage
  `X-AnchorMailbox`) plus les ajouts de ce fork (boutons de vote, dossiers masqués, `slow_sync`,
  répertoire, raccourcis de navigation), `ResolveNames` réuni.

Preuves de la fusion : `cargo xtask ci` vert (fmt, `clippy -D warnings`, 375 tests, layers, assets,
wasm) et un build `--workspace` complet. Après l'unification des dialogues, `cargo xtask ci` reste
vert (374 tests — un test de l'origine retiré avec ses commandes). La version du manifeste suit
l'origine (`0.19.0`).

### État de la récupération : v0.21, v0.22 et le découpage en petits fichiers (2026-10-22)

Les 51 commits de l'origine (`f0619af..1dc3ba7`) sont fusionnés en trois fusions, dans une
branche à part puis dans `main` : jusqu'à `d0dffbd` (les fonctions de la v0.22), puis le découpage
de l'origine en petits fichiers (`d0dffbd..f78afa9`), puis le reste (`..1dc3ba7`). L'historique de
l'origine est un parent de chaque fusion : la prochaine repartira de `1dc3ba7`.

**Méthode.**
- **Renommer l'origine d'abord.** Chaque commit de l'origine utilisé comme point de fusion est
  recopié avec MailCraft → Quatrelle appliqué aux chemins et au texte (comme `9bb5d25` l'avait
  fait ici), et la fusion se fait contre ces copies (`git merge-tree --merge-base`). Vérifié : le
  renommage automatique de `477b835` redonne `9bb5d25` à l'identique, hors ajouts voulus
  (`legacy.rs`, icônes). Il ne reste alors que les vrais conflits.
- **Le découpage de l'origine, élément par élément.** Quand l'origine éclate un fichier que ce fork
  a modifié, chaque fonction, méthode ou test du fork est fusionné à trois voies dans le fichier
  où l'origine l'a mis ; ce que le fork a ajouté va à côté de son voisin. Les commandes du fork vont
  dans les registres par domaine (`cmd/*.rs`), ses dialogues dans les groupes (`dialogs/*.rs`).
- **Rejouable.** Chaque étape est un script : le rejouer redonne exactement le même arbre. Les
  outils (Python, et `itemize`, un petit programme Rust qui liste les éléments d'un fichier) sont
  hors du dépôt, dans `~/mailcraft/merge-tools/`.

**Décisions.**
- **Gardé de ce fork** : l'apparence (ruban, rail, icônes Fluent, fenêtre qui suit le bureau), le
  calendrier et les contacts, et tout ce que ce fork avait déjà porté à la main (instance unique,
  en-tête étroit, modifications gardées pendant une synchro, IPv6/IPv4, semaine de travail, boîtes
  partagées, recherche sur le serveur EWS et IMAP), ainsi que la fiabilité Exchange (`5636c1c`).
- **Pris de l'origine** : Quick Steps (groupe « Actions rapides » du ruban, icône Fluent `flash`),
  expéditeurs bloqués et approuvés, Clean Up, dossiers de recherche, `larger:`/`smaller:`, rappels
  avec répétition et la fenêtre Rappels (à la place de la sonnerie simple de ce fork), plusieurs
  messages en cours de rédaction, réglages enregistrés au fil de l'eau, animations, recherche sur le
  serveur pour Graph et JMAP, portées de recherche et recherche avancée, Graph (ImmutableId, 429),
  la mise à jour automatique des versions de test — pointée sur `dgaudin/quatrelle`.
- **Les deux** : le `session/mailboxes.rs` de ce fork (autres boîtes ouvertes) devient
  `session/shared_mailboxes.rs`, l'origine ayant un module de ce nom. L'index de recherche de ce
  fork (construit par tranches) couvre les portées de l'origine ; l'onglet Rechercher garde ses
  boutons radio empilés, pour les quatre portées.
- **La règle des petits fichiers** de l'origine (`cargo xtask sizes` : 1000 lignes par fichier,
  16 fichiers par dossier) s'applique aussi à ce fork : les fichiers qu'il avait fait grossir sont
  découpés (`fab2ab9`), sans changement de comportement.

Preuves : `cargo xtask ci` vert à chaque étape (541 tests à la fin, fmt, `clippy -D warnings`,
tailles, couches, ressources, wasm), et la fenêtre regardée sur la boîte d'exemple.

### État de la récupération : reprises choisies de la v0.23 et de la v0.24 (2026-10-24)

L'origine a ajouté 60 commits après `1dc3ba7` (jusqu'à `49ad25c`). Ils ne sont pas fusionnés :
ils sont **repris un par un**, comme des *cherry-picks* des copies renommées
(`refs/merge/r-<commit>`, faites par `renamed.sh`), le diff entre la copie d'un commit et celle de
son parent appliqué à trois voies (`git apply -3`). La prochaine fusion complète repartira de
`1dc3ba7` : les changements repris à l'identique s'y retrouveront des deux côtés.

- **Repris** : les fuseaux horaires (`b9f0546`, `c4df00c`) et les invitations (`65d64b9`,
  `b34d158`, `4889315`) ; la revue de durcissement (`44dae91`, `014c2e6`, `9dffd7e`, `406fe3f`,
  `fd17e44`, `748b96d`, `e79ef90`, `1c0f7aa`, `303eda0`, `1a36d25`) ; le courrier plus sûr
  (`c433c66`, `d354be7`, `9954938`, `249b9f7`, `8912219`, `29a2459`) ; `mailto:` et les fichiers
  `.eml`/`.ics`/`.vcf` (`ebd873d`, `7db4066`, `f592eb9`, `0fbc402`) ; S/MIME de `a23eb9c` à
  `3866bdb`, puis `44d9506` sans ses lignes Windows.
- **S/MIME sans l'exception `unsafe`** (décision du propriétaire, 2026-10-24) : `1a30852` (le
  magasin de certificats Windows et le déchiffrement EC du trousseau macOS, seuls fichiers
  `unsafe` du crate `keys`) n'est pas repris, ni `897fb51` qui s'en sert. `crates/keys` garde les
  lints de l'espace de travail (`unsafe_code = "forbid"`). Signer, chiffrer, lire, les `.p12`, les
  cartes à puce (PKCS #11) et le trousseau macOS pour les clés RSA marchent.
- **Laissés pour la fusion complète** : les plafonds et IDLE d'IMAP (`7fbd05e`, `9083d2f`,
  `565f0bb`), qui visent la connexion IMAP que ce fork a remaniée ; le fuzz et `cargo-deny`
  (`9418429`, `5bf776e`) ; le découpage du worker et le renouvellement OAuth (`6b7fc80`,
  `d4eddd7`) ; les tests contre de vrais serveurs (`610c87c`) ; les numéros de version et la
  documentation de l'origine.
- **Déjà faits ici** : `7848644` (le fichier de données ouvert une seule fois) et `551f748` (la
  page Options dans son propre fichier).
- **Adaptations** : les rappels restent ceux de ce fork (dans `ical/alarms.rs`) ; les domaines
  connus des alertes viennent du carnet d'adresses de ce fork ; Chiffrer et Signer sont dans
  l'onglet Options du ruban du message, comme chez Outlook ; le `Keychain` reste
  `desktop/keychain.rs` ; quelques fichiers de ce fork passent en sous-dossier pour la règle des
  16 fichiers (`pim/scheduling.rs`, `addr/providers.rs`, `thread/voting.rs`,
  `desktop/appearance.rs`, `desktop/keychain.rs`), joignables par les mêmes chemins. Les
  certificats de test de l'origine gardent le nom « MailCraft Test CA » (signés, ils ne changent
  pas d'un octet).
- **Ajouté ici, côté Exchange** : le fuseau des séries lu et renvoyé (`StartTimeZone`, demandé
  nommément : `AllProperties` ne le donne pas, vérifié sur le serveur du propriétaire), et les
  champs d'un rendez-vous créé dans l'ordre du schéma EWS.


### État de la récupération : reprises choisies de la v0.25 et de la v0.26 (2026-10-26)

L'origine a ajouté 32 commits après `49ad25c` (jusqu'à `e46504c`). Même méthode : repris un par un
depuis les copies renommées, avec `~/mailcraft/merge-tools/pick.sh <commit>` (le diff de `refs/merge/r-<parent>` à
`refs/merge/r-<commit>`, appliqué par `git apply -3`).

- **Repris** : la synchronisation qui ne perd plus une modification faite après un renommage
  quand la liste des dossiers échoue (`e46504c`) ; S/MIME : pour quel certificat un message
  chiffré a été écrit (`eadd066`), les anciens certificats gardés pour lire le courrier qui leur
  était chiffré, listés dans Fichier › Informations › Sécurité, aussi dans le navigateur (`6a2eb5f`,
  `b8d4b9e`, `681542b`, `554901b`, `eac2ab2`, issue #6), et les signatures qui restent valides
  quand un relais retire `X-Mailer` (`47b89f1`).
- **Adaptations** : sans les boutons du magasin de certificats Windows et du trousseau macOS
  (`681542b`), ni les tests Windows ou contre de vrais serveurs, que ce fork n'a pas repris ; le
  bouton de carte à puce reste où il était.
- **Laissés** (décision du propriétaire, 2026-10-22 : l'apparence et le calendrier de ce fork
  restent) : le ruban « classique » et le mode sombre de l'origine (`b273cf8`, `b048d72`,
  `1cacf30`, issue #8) ; le serveur CalDAV donné à la main (`95c8779`, `695972d`, `fadbcc7`,
  `f6d66c5`, issue #7) ; le fuzz (`bab68ee`) ; la CI et la documentation de l'origine.
- **La mise à jour intégrée** (accord du propriétaire, 2026-10-26) : le téléchargement qui reprend
  où il s'était arrêté, la barre « prête » avec Redémarrer maintenant, la question à la fermeture,
  les AppImages là où l'utilisateur peut écrire (`2e4b4ed`, `e79f121`, `2b2ccb9`, `5e0620b`,
  `9e33e21`, `0533388`, `18a8a97`). Les fichiers du moteur de mise à jour sont ceux de l'origine,
  avec les versions de ce fork (`dgaudin/quatrelle`) ; la page À propos dans
  `backstage/about.rs`. `f6119f0` (les services du bureau dans `desktop.rs`) était déjà fait ici.
- **Les reprises laissées en v0.23 et v0.24, faites** (demande du propriétaire, 2026-10-26, avant
  d'y brancher Gmail et Yahoo) : les plafonds d'IMAP par commande (`7fbd05e` ; le téléchargement
  d'avance de ce fork, `sync/ahead.rs`, prend les mêmes lots bornés en taille), IDLE et la boîte de
  réception synchronisée à l'instant (`9083d2f`), le code de réponse suivi d'un texte non ASCII
  (`565f0bb`), le jeton OAuth renouvelé une fois avant de redemander la connexion (`d4eddd7`),
  cargo-deny (`5bf776e`), le fuzz (`9418429`, `bab68ee`, ses cibles IMAP et JMAP adaptées à ce
  fork) et les tests contre de vrais serveurs (`610c87c`). Le découpage du worker de l'origine
  (`6b7fc80`) n'est pas repris : celui de ce fork l'est déjà, autrement.

### État de la récupération : reprises choisies de la v0.27 (2026-10-27)

L'origine a ajouté 50 commits après `e46504c` (jusqu'à `d826960`). Même méthode, un par un depuis
les copies renommées (`~/mailcraft/merge-tools/pick.sh`, puis `run_picks.sh` qui applique,
compile et committe chaque reprise propre, et s'arrête au premier conflit).

- **Repris** : l'accessibilité (chaque contrôle dessiné a un nom pour les lecteurs d'écran ; les
  boutons propres à ce fork aussi, `633fd44`), la fermeture demandée même fenêtre réduite
  (`519dee1`, dans le `logic` de ce fork, qui lit l'état de la mise à jour d'abord), ⌘Q sur Mac
  (`3eab186`), le cache qui réécrit un fichier tenu un instant par un autre programme (`aec0eb1`,
  `f436878`), les dossiers relatifs ignorés (`7a7466b`), les noms de périphériques Windows
  (`7880728`), la boîte de dialogue qui rouvre le dernier dossier (`bab188a`), les images et pages
  PDF jamais plus grandes que la carte graphique (`4a04574`), le défilement du ruban par onglet
  (`e0c2d64`), le curseur au-delà du texte (`ebd2856`), l'hébreu et l'arabe dans les champs
  (`25e6033`, `ea13b4e`), sélectionnés et copiés dans un message (`8b77776`), les paragraphes de
  droite à gauche contre le bord droit (`754ab47`, en texte brut aussi), la place d'une image dans
  le texte (`156cd84`), la barre d'état aux nombreux comptes en attente (`fcdfbff`), le test « au
  hasard » de la fenêtre (`0d830b1`), des traductions (`c63c587`, `7800e8d`), l'exemple rempli de
  nouveau (`e3ccec5`), l'identité JMAP créée quand il n'y en a pas (`37a5ff0`), l'empaquetage et les
  polices du navigateur (`ff097b1`, `f1e7465`, `8e7d9b8`, `ccfacd8`, `8d9679b`, `8955bb3`,
  `3927f7e`, `42eba80`, `4799f8a`, `7342667`, `f95d2f3`, `82542f4`).
- **winit et egui partagés** : winit vient de `storytold/winit` (révision `5506b1a9`, la copie de
  `vendor/` est retirée, `4177c89`), egui de `storytold/egui` (`8f6d82d`), comme les autres
  applications Craft. Le winit partagé apporte le dépôt de fichiers sous Wayland (W15).
- **Adaptés** : le User-Agent propre à un compte Exchange et le nom d'utilisateur à saut de ligne
  refusé sont repris de `9ade2bb` sans sa recherche dans l'annuaire, ce fork ayant la sienne
  (Vérifier les noms, Liste d'adresses globale, l'autre adresse du champ De) ; le User-Agent va
  aussi sur les requêtes propres à ce fork (boîtes partagées, notifications).
- **Rattrapé** : `ca7c5e6` (de la v0.26, passé inaperçu alors), dont `ccfacd8` avait besoin : il
  donne à `GLIBC_FLOOR` sa valeur par défaut (2.35). Sans lui, le .rpm des tests 78 à 80 demandait
  `libc.so.6(GLIBC_)(64bit)`, que rien ne fournit, et dnf le refusait. `package.sh` refuse
  désormais toute variable de `nfpm.yaml` restée vide.
- **Laissés** : la mise en miroir de la fenêtre en arabe et en hébreu (`d2e280e` : elle réécrit la
  liste, les dossiers, la lecture et la rédaction, que ce fork dessine à sa façon) ; la couleur
  d'accent, la barre du haut sobre et les bordures (`cf15073`, l'apparence de ce fork reste) ;
  Kerberos (`c2f1788` : il réécrit la boucle de connexion EWS, que ce fork a renforcée autrement,
  et c'est le chemin NTLM du propriétaire) ; l'annuaire de Microsoft 365 (`81fa8d4`, posé sur le
  moteur d'annuaire de l'origine) ; les certificats gardés dans le navigateur (`41f42af`) ;
  l'agenda et les contacts EWS lus séparément (de `9ade2bb` : l'agenda de ce fork est le sien) ;
  ce qui était déjà fait ici (`500e2fc`, le thème du bureau ; `217325c`, Ctrl+V d'une image) ; la
  CI, la version et la documentation de l'origine (`2d8b274`, `baa180a`, `daadfd6`, `db8851c`,
  `763cb9f`, `d826960`).

### État de la récupération : v0.28 (2026-10-28)

L'origine a publié la v0.28 : onze commits après `d826960`. Même méthode, un par un.

- **Repris tels quels** (le jour même) : l'empaquetage et la CI (`df0f10a`, l'icône de Quatrelle
  épinglée gardée après une mise à jour sous Windows ; `6df62e5`, une version sans craft-fonts
  refusée ; `3c2a9fa`, libxkbcommon-x11 dans le .deb et le .rpm ; `7dc8276`, les panneaux
  d'ouverture et d'enregistrement du Mac dans la langue de l'interface ; `7ce73f5`, le certificat
  de signature décodé dans un dossier privé ; `0619fe9`, l'écriture réservée au travail qui
  rédige la version ; `b21aa6a`, FreeBSD et Windows ARM64 aux seules publications et chaque
  semaine).
- **Adapté** : l'abonnement à un calendrier par son lien iCal ou `webcal://` (`29f1bec`, Ajouter
  un calendrier › Depuis Internet d'Outlook). Le modèle (`WebCalendar`, `calendar_link`), le
  protocole (`net/protocols/web_calendar.rs`, ETag et Last-Modified), les commandes
  (`calendar.subscribe`, `calendar.unsubscribe`, `calendar.refreshSubscribed`), la boîte de
  dialogue et la ligne du volet (lien et état au survol, Actualiser et Supprimer au clic droit)
  viennent de l'origine ; la lecture tourne dans `worker/pool.rs` de ce fork (l'origine a
  `threads.rs`). Décisions du propriétaire, dans le sillage d'Outlook : un lien vers un serveur
  du réseau local est permis (le lecteur `http::send`, pas celui des images réservé à
  l'Internet) ; un calendrier abonné **n'est pas** un calendrier du compte (`own_calendar`),
  donc ses rendez-vous sont mis de côté avec ceux des autres hors du calendrier (`parking.rs` ;
  une lecture survenue pendant qu'ils étaient de côté l'emporte au retour) et ses alarmes ne
  sonnent jamais ; le point d'entrée du ruban est un menu Ajouter un calendrier (Depuis le
  carnet d'adresses… — l'ancien bouton Ouvrir un calendrier — et Depuis Internet…), avec le
  lien S'abonner à un calendrier… du volet. La fusion de synchronisation de ce fork
  (`syncing.rs`) garde le calendrier abonné et ses rendez-vous à travers une synchronisation
  complète EWS/CalDAV, même quand un rendez-vous du serveur porte le même UID, et une occurrence
  publiée sans sa série n'est pas jetée comme orpheline. Le refus de supprimer dans un
  calendrier en lecture seule, que l'origine corrige au passage, était déjà là.
- **Adapté** : de `2f57946` (les personnes trouvées dans l'annuaire montrées par leur nom
  complet), seule la moitié moteur : un contact connu ici par sa seule adresse (ou nommé par
  elle, ou par sa partie utilisateur) prend le nom que donne l'annuaire (`fill_name`), dans
  Vérifier les noms — qui interroge désormais l'annuaire pour un tel contact au lieu de
  l'écrire aussitôt, comme Outlook consulte d'abord la liste d'adresses — et dans l'autre
  adresse du champ De ; un nom qu'on a donné soi-même est gardé.
- **Laissés** : la moitié EWS de `2f57946` (`ReturnFullContactData`, le nom d'affichage
  préféré au nom de la boîte) : ce fork a sa propre recherche dans l'annuaire, et le serveur du
  propriétaire nomme déjà les gens correctement ; la documentation et la version de l'origine
  (`70b6257`, `3345a18`).

## 7. La rédaction des messages, rapprochée d'Outlook (2026-10-22)

Quatre chantiers, chacun dans son commit, à la suite d'un audit de l'éditeur HTML.

**Les signatures gardent leur mise en forme.** La signature était relue depuis son HTML à chaque
message (`signature_html`), ce qui perdait les tailles, les polices, le soulignement et le
centrage. Elle est désormais enregistrée telle que l'éditeur la tient (`AccountConfig::signature_styles`,
les `rich::Styles` en JSON), et l'HTML n'est relu que pour les signatures d'avant — en gardant
cette fois les polices (`fonts::mark`/`apply`), et en appliquant `inline_css` avant `clean_html`
aux signatures écrites en HTML (les styles par classe d'Outlook passaient à la trappe). La ligne
`-- ` qu'ajoutait Quatrelle avant chaque signature est devenue une option, désactivée par défaut
(Outlook n'en met pas) : Options › « Placer une ligne « -- » avant la signature »
(`Prefs::signature_dashes`, commande `view.signatureDashes`).

**Le correcteur orthographique.** Le message entier était vérifié plusieurs fois par image, et les
suggestions recalculées à chaque image tant que le menu restait ouvert. Désormais (`spellcheck.rs`) :
- la vérification se fait paragraphe par paragraphe, mémorisée par le texte du paragraphe ;
  les suggestions sont calculées une fois, à l'ouverture du menu ou du dialogue ;
- ni la signature ni l'original cité d'une réponse ne sont vérifiés ; un mot dans une écriture
  sans dictionnaire (chinois, arabe…) n'est pas souligné ;
- les suggestions FR/EN sont fusionnées et classées par distance d'édition, la langue active
  d'abord (`Spell::suggest_in`) ; elles sont en tête du menu contextuel, avec « Ignorer partout »
  (pour le message) et « Ajouter au dictionnaire » ;
- la langue se choisit par message (Révision › Langue : chaque dictionnaire, ou toutes les
  langues ; par défaut celle de l'interface), commande `compose.language` ; les dictionnaires
  Hunspell du système (`/usr/share/hunspell`, `/usr/share/myspell` : `de_DE`, `es_ES`…) sont lus
  au démarrage, jamais embarqués ;
- un mot personnel commençant par une majuscule accentuée (« Été ») est reconnu (comparaison en
  minuscules Unicode, non ASCII).

**Le ruban du message** (`rich_edit/ribbon.rs`) remplace la rangée de boutons et la seconde barre
d'outils : les onglets Message | Insertion | Options | Format du texte | Révision, avec les
widgets du ruban principal (`Groups`, `big_button`, `big_more`, `stack`). Enregistrer, Détacher
et Abandonner sont de petits boutons à droite des onglets ; le grand bouton Envoyer reste à gauche
de De/À/Cc/Objet. Nouveaux boutons : Vérifier les noms (commande `compose.checkNames`, qui
complète les adresses depuis les contacts), Ligne horizontale, choix de la police, Coller › Ne
garder que le texte. Les nouvelles icônes sont des SVG Fluent inchangés (`assets/icons/fluent/`,
une ligne chacune dans `ATTRIBUTION.md`).

**La fidélité de l'éditeur.** Les polices choisies (Arial, Calibri…) sont lues sur le système
(`Services::typeface`, fontconfig sous Linux, avec la police de substitution — Liberation Sans
pour Arial) et dessinées dans l'éditeur (`look/typefaces.rs`) ; les suites d'espaces et les
tabulations partent en `&nbsp;` (elles s'effondraient chez le destinataire) ; les paragraphes
centrés ou alignés à droite le sont sur toutes leurs lignes ; ⌘⇧Z refait sur Mac au lieu
d'effacer la mise en forme ; Couper et Copier mettent aussi l'HTML dans le presse-papiers
(`set_clipboard_html_writer`, arboard) ; le HTML collé garde ses images incorporées (`data:`)
et celles que le message possède déjà (`cid:`), en laissant les autres.
